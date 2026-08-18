use super::*;
use prod_ir::parser::parse_module;

fn generate(ir: &str) -> String {
    let (_, module) = parse_module(ir).unwrap();
    generate_module(&module).unwrap()
}

fn generate_err(ir: &str) -> Error {
    let (_, module) = parse_module(ir).unwrap();
    generate_module(&module).unwrap_err()
}

#[test]
fn test_generate_c_header_and_matching_scalar_wrappers() {
    let ir = r#"
(module Demo
  (def add ((a Nat) (b Nat)) Nat (add a b))
  (def less ((a Nat) (b Nat)) Bool (lt a b))
  (def echo ((value Bool)) Bool value)
  (def riskyFlag ((x Nat)) Bool (let n (add x 1) (lt n 2)))
)
"#;
    let (_, module) = parse_module(ir).unwrap();
    let bindings = generate_c_bindings(&module).unwrap();
    assert!(bindings.header.contains("#include <stdint.h>"));
    assert!(bindings
        .header
        .contains("prod_demo_add_result_t prod_demo_add(uint64_t a, uint64_t b);"));
    assert!(bindings.header.contains(
        "typedef struct prod_demo_add { prod_status_t status; uint64_t value; } prod_demo_add_result_t;"
    ));
    assert!(bindings
        .header
        .contains("uint8_t prod_demo_less(uint64_t a, uint64_t b);"));
    assert!(bindings
        .header
        .contains("uint8_t prod_demo_echo(uint8_t value);"));

    // The Rust adapter uses Rust ABI types internally and converts bool at
    // the boundary, so the file can be included in an unsafe-forbidden crate.
    assert!(bindings.rust.contains(
        "pub extern \"C\" fn prod_demo_add(a: u64, b: u64) -> ProdFfi_prod_demo_add_Result"
    ));
    assert!(bindings.rust.contains("add(a, b)"));
    assert!(bindings
        .rust
        .contains("pub extern \"C\" fn prod_demo_less(a: u64, b: u64) -> u8"));
    assert!(bindings.rust.contains("if less(a, b) { 1 } else { 0 }"));
    assert!(bindings
        .rust
        .contains("pub extern \"C\" fn prod_demo_echo(value: u8) -> u8"));
    assert!(bindings.rust.contains("echo(value != 0)"));
    assert!(bindings
        .header
        .contains("prod_demo_riskyflag_result_t prod_demo_riskyflag(uint64_t x);"));
    assert!(bindings
        .rust
        .contains("pub value: u8,\n}\n\n#[no_mangle]\npub extern \"C\" fn prod_demo_riskyflag"));
    assert!(bindings.rust.contains("value: 0"));
}

#[test]
fn test_c_header_omits_list_abi_without_guessing_ownership() {
    let ir = r#"
(module M
  (def digits ((n Nat)) (List Nat) (ctor "List.nil"))
  (def scalar () Nat 7)
)
"#;
    let (_, module) = parse_module(ir).unwrap();
    let bindings = generate_c_bindings(&module).unwrap();
    assert!(bindings
        .header
        .contains("Definitions omitted from this scalar ABI"));
    assert!(bindings.header.contains("uint64_t prod_m_scalar(void);"));

    let only_list = r#"
(module M
  (def digits ((n Nat)) (List Nat) (ctor "List.nil"))
)
"#;
    let (_, module) = parse_module(only_list).unwrap();
    assert!(matches!(
        generate_c_bindings(&module),
        Err(CAbiError::UnsupportedDefinition { .. })
    ));
}

#[test]
fn test_generate_multi_language_sdk_bundle_from_one_abi() {
    let ir = r#"
(module Demo
  (def add ((a Nat) (b Nat)) Nat (add a b))
  (def less ((a Nat) (b Nat)) Bool (lt a b))
)
"#;
    let (_, module) = parse_module(ir).unwrap();
    let sdk = generate_sdks(&module, "demo").unwrap();

    assert!(sdk.c_header.contains("prod_demo_add_result_t"));
    assert!(sdk.c_wrapper.contains("extern \"C\" fn prod_demo_add"));
    assert!(sdk.rust.contains("#[link(name = \"demo\")]"));
    assert!(sdk.rust.contains("pub fn prod_demo_less"));
    assert!(sdk.python.contains("import ctypes"));
    assert!(sdk.python.contains("def prod_demo_add(lib, a, b):"));
    assert!(sdk.typescript.contains("export function bind"));
    assert!(sdk.typescript.contains("prod_demo_less"));
    assert!(sdk.kotlin.contains("import com.sun.jna.Library"));
    assert!(sdk.kotlin.contains("interface Lean4ProdNative"));
    assert!(sdk.wasm.contains("use wasm_bindgen::prelude::*"));
    assert!(sdk
        .wasm
        .contains("#[wasm_bindgen(js_name = \"prod_demo_add\")]"));
    assert!(sdk.wasm.contains("fn __prod_compute_error"));
    assert!(sdk.kotlin.contains("Result<Long>"));
    assert!(!sdk.kotlin.contains("Result<Long, Int>"));
}

#[test]
fn test_generate_class_index() {
    let ir = r#"
(module UorAtlas.Kernel
  (type "UorAtlas.Instance"
    (ctor "UorAtlas.Instance.mk" (q Nat) (T Nat) (O Nat)))

  (def classIndex ((h2 Nat) (d Nat) (l Nat) (inst (named "UorAtlas.Instance"))) Nat
    (add (mul (call stride inst) h2)
         (add (mul (proj "UorAtlas.Instance" "O" inst) d) l)))

  (def belt ((inst (named "UorAtlas.Instance"))) Nat
    (mul (call class_count inst)
         (shl 1 (sub (proj "UorAtlas.Instance" "O" inst) 1))))
)
"#;
    let out = generate(ir);
    assert_eq!(
        out,
        "#[derive(Debug, Clone, Copy, PartialEq, Eq)]\npub struct Instance {\n    pub q: u64,\n    pub T: u64,\n    pub O: u64,\n}\n\npub fn classIndex(h2: u64, d: u64, l: u64, inst: crate::Instance) -> Result<u64, crate::ComputeError> {\n    Ok(((((stride(inst)) as u64).checked_mul(h2).ok_or(crate::ComputeError::MulOverflow)?) as u64).checked_add((((((inst).O) as u64).checked_mul(d).ok_or(crate::ComputeError::MulOverflow)?) as u64).checked_add(l).ok_or(crate::ComputeError::AddOverflow)?).ok_or(crate::ComputeError::AddOverflow)?)\n}\n\npub fn belt(inst: crate::Instance) -> Result<u64, crate::ComputeError> {\n    Ok(((class_count(inst)) as u64).checked_mul(((1) as u64).checked_shl(u32::try_from((((inst).O) as u64).saturating_sub(1)).map_err(|_| crate::ComputeError::ShiftExponentTooLarge)?).ok_or(crate::ComputeError::ShiftOverflow)?).ok_or(crate::ComputeError::MulOverflow)?)\n}\n\n"
    );
}

#[test]
fn test_generate_match() {
    let ir = r#"
(module M
  (def f ((x Nat)) Nat
    (cases x
      (alt "Some" (v) v)
      (default 0)))
)
"#;
    let out = generate(ir);
    assert_eq!(
        out,
        "pub fn f(x: u64) -> u64 {\n    match x {\n        Some(v) => v,\n        _ => 0,\n    }\n}\n\n"
    );
}

#[test]
fn test_generate_list_param_is_a_slice_and_return_is_a_buffer() {
    // Lean List: a parameter borrows as `&[T]` and matches with slice
    // patterns; a return builds into a caller-owned `&mut [T]`.
    let ir = r#"
(module M
  (def digitSum ((xs (List Nat))) Nat
    (cases xs
      (alt "List.nil" () 0)
      (alt "List.cons" (h t) (add h (call digitSum t)))))
  (def digits ((n Nat)) (List Nat)
    (if (lt n 8)
        (ctor "List.cons" n (ctor "List.nil"))
        (ctor "List.cons" (mod n 8) (call digits (div n 8)))))
)
"#;
    let out = generate(ir);
    assert_eq!(
        out,
        "pub fn digitSum(xs: &[u64]) -> Result<u64, crate::ComputeError> {\n    Ok(match xs {\n        [] => 0,\n        [h, t @ ..] => { let h = *h; ((h) as u64).checked_add(digitSum(t)?).ok_or(crate::ComputeError::AddOverflow)? },\n    })\n}\n\npub fn digits(n: u64, output: &mut [u64]) -> Result<usize, crate::ComputeError> {\n    if (n < 8) { match (output).split_first_mut() { None => Err(crate::ComputeError::OutputTooSmall), Some((__head0, __rest0)) => { *__head0 = n; let __len0 = Ok::<usize, crate::ComputeError>(0)?; Ok(__len0 + 1) } } } else { match (output).split_first_mut() { None => Err(crate::ComputeError::OutputTooSmall), Some((__head0, __rest0)) => { *__head0 = if (8) == 0 { 0 } else { (n) % (8) }; let __len0 = digits(if (8) == 0 { 0 } else { (n) / (8) }, __rest0)?; Ok(__len0 + 1) } } }\n}\n\n"
    );
}

#[test]
fn test_generate_list_builder_resolves_anf_let_bindings() {
    // The exact shape prod-export emits for `digits`: LCNF is in A-normal
    // form, so every cons cell arrives as a `let`. Those bindings have no
    // runtime representation in buffer mode — they are resolved through the
    // builder environment, not materialized.
    let ir = r#"
(module UorAtlas.Kernel
  (type "UorAtlas.Instance"
    (ctor "UorAtlas.Instance.mk" (q Nat) (T Nat) (O Nat)))

  (def digits ((fuel Nat) (n Nat) (i (named "UorAtlas.Instance"))) (List Nat)
    (cases fuel
      (alt "Nat.zero" () (let _x_46 (ctor "List.nil") _x_46))
      (alt "Nat.succ" (n_25)
        (let _x_47 (proj "UorAtlas.Instance" "O" i)
          (if (lt n _x_47)
            (let _x_55 (ctor "List.nil") (let _x_56 (ctor "List.cons" n _x_55) _x_56))
            (let _x_50 (mod n _x_47)
              (let _x_51 (div n _x_47)
                (let _x_52 (call digits n_25 _x_51 i)
                  (let _x_53 (ctor "List.cons" _x_50 _x_52) _x_53)))))))))
)
"#;
    let out = generate(ir);
    assert_eq!(
        out,
        "#[derive(Debug, Clone, Copy, PartialEq, Eq)]\npub struct Instance {\n    pub q: u64,\n    pub T: u64,\n    pub O: u64,\n}\n\npub fn digits(fuel: u64, n: u64, i: crate::Instance, output: &mut [u64]) -> Result<usize, crate::ComputeError> {\n    match fuel {\n        0 => Ok::<usize, crate::ComputeError>(0),\n        _ => { let n_25 = (fuel).saturating_sub(1); { let _x_47 = (i).O; if (n < _x_47) { match (output).split_first_mut() { None => Err(crate::ComputeError::OutputTooSmall), Some((__head0, __rest0)) => { *__head0 = n; let __len0 = Ok::<usize, crate::ComputeError>(0)?; Ok(__len0 + 1) } } } else { { let _x_50 = if (_x_47) == 0 { 0 } else { (n) % (_x_47) }; { let _x_51 = if (_x_47) == 0 { 0 } else { (n) / (_x_47) }; match (output).split_first_mut() { None => Err(crate::ComputeError::OutputTooSmall), Some((__head0, __rest0)) => { *__head0 = _x_50; let __len0 = digits(n_25, _x_51, i, __rest0)?; Ok(__len0 + 1) } } } } } } },\n    }\n}\n\n"
    );
}

#[test]
fn test_generate_zero_arg_list_golden_is_a_promoted_static_slice() {
    let ir = r#"
(module UorAtlas.Goldens
  (def golden_digits_43_canonical () (List Nat)
    (ctor "List.cons" 3 (ctor "List.cons" 5 (ctor "List.nil"))))
)
"#;
    let out = generate(ir);
    assert_eq!(
        out,
        "pub fn golden_digits_43_canonical() -> &'static [u64] {\n    &[3, 5]\n}\n\n"
    );
}

#[test]
fn test_fallibility_is_precise_not_uniform() {
    // Only definitions that can actually fail carry `Result`, and that
    // property propagates through the call graph — including recursion.
    let ir = r#"
(module M
  (def pure ((x Nat) (y Nat)) Nat (sub x y))
  (def viaDiv ((x Nat)) Nat (call pure (div x 2) 1))
  (def risky ((x Nat)) Nat (add x 1))
  (def caller ((x Nat)) Nat (call risky x))
  (def loops ((fuel Nat) (x Nat)) Nat
    (cases fuel
      (alt "Nat.zero" () x)
      (alt "Nat.succ" (k) (call loops k (add x 1)))))
)
"#;
    let out = generate(ir);
    assert!(out.contains("pub fn pure(x: u64, y: u64) -> u64 {"));
    assert!(out.contains("pub fn viaDiv(x: u64) -> u64 {"));
    assert!(out.contains("pub fn risky(x: u64) -> Result<u64, crate::ComputeError> {"));
    assert!(out.contains("pub fn caller(x: u64) -> Result<u64, crate::ComputeError> {"));
    assert!(out.contains("Ok(risky(x)?)"));
    // A recursive definition reaches its own fixpoint.
    assert!(out.contains("pub fn loops(fuel: u64, x: u64) -> Result<u64, crate::ComputeError> {"));
    assert!(out.contains("loops(k, ((x) as u64).checked_add(1)"));
}

#[test]
fn test_intermediate_list_value_is_a_codegen_error() {
    // A list that flows anywhere other than the output buffer would need an
    // owned representation; fail honestly instead of allocating one.
    let ir = r#"
(module M
  (def digitSum ((xs (List Nat))) Nat
    (cases xs
      (alt "List.nil" () 0)
      (alt "List.cons" (h t) h)))
  (def digits ((n Nat)) (List Nat) (ctor "List.nil"))
  (def total ((n Nat)) Nat (call digitSum (call digits n)))
)
"#;
    assert_eq!(
        generate_err(ir),
        Error::UnsupportedList(
            "`digits` returns a list; its result cannot be used as an intermediate value"
                .to_string()
        )
    );
}

#[test]
fn test_nested_list_type_is_a_codegen_error() {
    let ir = r#"
(module M
  (def f ((x Nat)) (Option (List Nat)) (ctor "Option.none"))
)
"#;
    assert!(matches!(generate_err(ir), Error::UnsupportedList(_)));
}

#[test]
fn test_vec_type_is_rejected_as_heap_allocating() {
    let ir = r#"
(module M
  (def f ((xs (Vec Nat))) Nat 0)
)
"#;
    assert_eq!(generate_err(ir), Error::HeapType("(Vec u64)".to_string()));
}

#[test]
fn test_computed_zero_arg_list_is_a_codegen_error() {
    // A golden whose elements are computed cannot be a promoted static slice.
    let ir = r#"
(module M
  (def g () (List Nat) (ctor "List.cons" (add 1 2) (ctor "List.nil")))
)
"#;
    assert!(matches!(generate_err(ir), Error::UnsupportedList(_)));
}

#[test]
fn test_generate_option_and_bool() {
    // Option/Bool ctors and match arms map to Rust's native types.
    let ir = r#"
(module M
  (def tryDecode ((idx Nat)) (Option Nat)
    (if (le idx 96) (ctor "Option.some" idx) (ctor "Option.none")))
  (def fromOpt ((x (Option Nat))) Bool
    (cases x
      (alt "Option.some" (v) (ctor "Bool.true"))
      (alt "Option.none" () (ctor "Bool.false"))))
)
"#;
    let out = generate(ir);
    assert_eq!(
        out,
        "pub fn tryDecode(idx: u64) -> Option<u64> {\n    if (idx <= 96) { Some(idx) } else { None }\n}\n\npub fn fromOpt(x: Option<u64>) -> bool {\n    match x {\n        Some(v) => true,\n        None => false,\n    }\n}\n\n"
    );
}

#[test]
fn test_generate_nat_cases_recursion() {
    // LCNF structural recursion on Nat: `Nat.zero` → literal `0` pattern,
    // `Nat.succ k` → `_` arm with the predecessor bound via saturating_sub.
    let ir = r#"
(module M
  (def digitCount ((fuel Nat) (n Nat)) Nat
    (cases fuel
      (alt "Nat.zero" () 0)
      (alt "Nat.succ" (k) (if (lt n 8) 1 (add 1 (call digitCount k (div n 8)))))))
)
"#;
    let out = generate(ir);
    assert_eq!(
        out,
        "pub fn digitCount(fuel: u64, n: u64) -> Result<u64, crate::ComputeError> {\n    Ok(match fuel {\n        0 => 0,\n        _ => { let k = (fuel).saturating_sub(1); if (n < 8) { 1 } else { ((1) as u64).checked_add(digitCount(k, if (8) == 0 { 0 } else { (n) / (8) })?).ok_or(crate::ComputeError::AddOverflow)? } },\n    })\n}\n\n"
    );
}

#[test]
fn test_generate_ctor_proj() {
    let ir = r#"
(module M
  (def f ((x Nat)) Nat
    (proj "Pair" "fst" (ctor "Pair" x 2)))
)
"#;
    let out = generate(ir);
    assert_eq!(
        out,
        "pub fn f(x: u64) -> u64 {\n    (Pair(x, 2)).fst\n}\n\n"
    );
}

#[test]
fn test_generate_projection_uses_field_names() {
    let ir = r#"
(module UorAtlas.Kernel
  (type "UorAtlas.Instance"
    (ctor "UorAtlas.Instance.mk" (q Nat) (T Nat) (O Nat)))
  (def decode ((i (named "UorAtlas.Instance"))) (Tuple Nat (Tuple Nat Nat))
    (ctor "Prod.mk" (proj "UorAtlas.Instance" "q" i)
      (ctor "Prod.mk" (proj "UorAtlas.Instance" "O" i) 1)))
)
"#;
    let out = generate(ir);
    assert!(out.contains("((i).q, ((i).O, 1))"));
}

#[test]
fn test_projection_of_keyword_field_is_raw_escaped() {
    let ir = r#"
(module M
  (type "M.Rec" (ctor "M.Rec.mk" (type Nat)))
  (def get ((r (named "M.Rec"))) Nat (proj "M.Rec" "type" r)))
"#;
    let out = generate(ir);
    assert!(out.contains("(r).r#type"));
}

#[test]
fn test_generate_kernel_ir_shapes() {
    // The exact def shapes prod-export emits for stride and classDecode
    // (see rust/prod-core/kernel.ir).
    let ir = r#"
(module UorAtlas.Kernel
  (type "UorAtlas.Instance"
    (ctor "UorAtlas.Instance.mk" (q Nat) (T Nat) (O Nat)))

  (def stride ((i (named "UorAtlas.Instance"))) Nat
    (let _x_4 (proj "UorAtlas.Instance" "T" i) (let _x_5 (proj "UorAtlas.Instance" "O" i) (let _x_13 (mul _x_4 _x_5) _x_13))))

  (def classDecode ((idx Nat) (i (named "UorAtlas.Instance"))) (Tuple Nat (Tuple Nat Nat))
    (let _x_4 (call stride i) (let h2 (div idx _x_4) (let rem (mod idx _x_4) (let _x_10 (proj "UorAtlas.Instance" "O" i) (let d (div rem _x_10) (let l (mod rem _x_10) (let _x_13 (ctor "Prod.mk" d l) (let _x_14 (ctor "Prod.mk" h2 _x_13) _x_14)))))))))
)
"#;
    let out = generate(ir);
    assert_eq!(
        out,
        "#[derive(Debug, Clone, Copy, PartialEq, Eq)]\npub struct Instance {\n    pub q: u64,\n    pub T: u64,\n    pub O: u64,\n}\n\npub fn stride(i: crate::Instance) -> Result<u64, crate::ComputeError> {\n    Ok({ let _x_4 = (i).T; { let _x_5 = (i).O; { let _x_13 = ((_x_4) as u64).checked_mul(_x_5).ok_or(crate::ComputeError::MulOverflow)?; _x_13 } } })\n}\n\npub fn classDecode(idx: u64, i: crate::Instance) -> Result<(u64, (u64, u64)), crate::ComputeError> {\n    Ok({ let _x_4 = stride(i)?; { let h2 = if (_x_4) == 0 { 0 } else { (idx) / (_x_4) }; { let rem = if (_x_4) == 0 { 0 } else { (idx) % (_x_4) }; { let _x_10 = (i).O; { let d = if (_x_10) == 0 { 0 } else { (rem) / (_x_10) }; { let l = if (_x_10) == 0 { 0 } else { (rem) % (_x_10) }; { let _x_13 = (d, l); { let _x_14 = (h2, _x_13); _x_14 } } } } } } } })\n}\n\n"
    );
}

#[test]
fn test_undeclared_dotted_ctor_is_rejected_not_rendered_as_a_path() {
    // This used to render `(Unknown.Struct(x)).count` and exit 0. A dotted
    // Lean name is not a Rust path in expression position — `A.B(x)` parses
    // as a field access on a value named `A` followed by a call, so even
    // `syn::parse_str` accepts it and the breakage lands in rustc, far from
    // the IR that caused it. It is now an `UnresolvedCall` naming the ctor.
    let ir = r#"
(module M
  (def f ((x Nat)) Nat
    (proj "Unknown.Struct" "count" (ctor "Unknown.Struct" x)))
)
"#;
    assert_eq!(
        generate_err(ir),
        Error::UnresolvedCall("Unknown.Struct".to_string())
    );
}

#[test]
fn test_undeclared_dot_free_ctor_still_renders_as_a_bare_path() {
    // The complement of the test above: a dot-free constructor name is at
    // least a syntactically valid Rust path, so the bare-name fallthrough is
    // left intact for hosts that supply the type by hand.
    let ir = r#"
(module M
  (def f ((x Nat)) Nat (ctor "Pair" x 2))
)
"#;
    assert!(generate(ir).contains("Pair(x, 2)"));
}

#[test]
fn test_ctor_in_a_definition_body_only_is_rejected_when_undeclared() {
    // The end-to-end shape of the bug: a definition whose body constructs and
    // projects a type that is not declared in the module. `Lower.lean` now
    // declares body-reachable types (`declTypeNames`), so this IR is what
    // reaches codegen only when the declaration really is missing — and then
    // it must fail, not emit `Conformance.NoProp.mk(n, n)`.
    let ir = r#"
(module Conformance
  (def c_ctor_body_only ((n Nat)) Nat
    (let _x_1 (ctor "Conformance.NoProp.mk" n n)
      (let _x_2 (proj "Conformance.NoProp" "alpha" _x_1) _x_2)))
)
"#;
    assert_eq!(
        generate_err(ir),
        Error::UnresolvedCall("Conformance.NoProp.mk".to_string())
    );
}

#[test]
fn test_ctor_in_a_definition_body_only_renders_when_declared() {
    // The same IR with the declaration `Lower.lean` now emits for it.
    let ir = r#"
(module Conformance
  (type "Conformance.NoProp"
    (ctor "Conformance.NoProp.mk" (alpha Nat) (beta Nat)))
  (def c_ctor_body_only ((n Nat)) Nat
    (let _x_1 (ctor "Conformance.NoProp.mk" n n)
      (let _x_2 (proj "Conformance.NoProp" "alpha" _x_1) _x_2)))
)
"#;
    let out = generate(ir);
    assert!(out.contains("pub struct NoProp {"));
    assert!(out.contains("crate::NoProp { alpha: n, beta: n }"));
    assert!(out.contains("(_x_1).alpha"));
}

/// Every `Error` variant must appear in the published rejection table
/// (`REJECTIONS`, rendered into `specs/lean-for-production.md`). The two are
/// separate lists that have to agree, and nothing but this test makes them:
/// a new variant would otherwise vanish from the contract while
/// `just subset-check` still passed.
///
/// The match below is exhaustive on purpose — no wildcard arm — so adding an
/// `Error` variant is a *compile* error here, not a silently-passing test.
#[test]
fn test_every_error_variant_is_published_in_rejections() {
    let s = || String::from("x");
    let all = [
        Error::OpaqueExpr(s()),
        Error::ParamOutOfBounds(0),
        Error::UnsupportedList(s()),
        Error::HeapType(s()),
        Error::RecursiveType(s()),
        Error::PolymorphicType(s()),
        Error::UnsupportedFieldType(s()),
        Error::DuplicateTypeName(s()),
        Error::OpaqueType(s()),
        Error::UnresolvedCall(s()),
        Error::UnknownField(s(), s()),
        Error::UnsupportedJoinPoint(s()),
    ];

    for error in &all {
        let name = match error {
            Error::OpaqueExpr(_) => "OpaqueExpr",
            Error::ParamOutOfBounds(_) => "ParamOutOfBounds",
            Error::UnsupportedList(_) => "UnsupportedList",
            Error::HeapType(_) => "HeapType",
            Error::RecursiveType(_) => "RecursiveType",
            Error::PolymorphicType(_) => "PolymorphicType",
            Error::UnsupportedFieldType(_) => "UnsupportedFieldType",
            Error::DuplicateTypeName(_) => "DuplicateTypeName",
            Error::OpaqueType(_) => "OpaqueType",
            Error::UnresolvedCall(_) => "UnresolvedCall",
            Error::UnknownField(..) => "UnknownField",
            Error::UnsupportedJoinPoint(_) => "UnsupportedJoinPoint",
        };
        assert!(
            REJECTIONS.iter().any(|(variant, _)| *variant == name),
            "`Error::{}` is not in REJECTIONS, so the published subset contract does not disclose it",
            name
        );
    }

    // ...and no entry in REJECTIONS without a matching variant.
    assert_eq!(
        REJECTIONS.len(),
        all.len(),
        "REJECTIONS lists {} rejections for {} Error variants",
        REJECTIONS.len(),
        all.len()
    );
}

#[test]
fn test_unsupported_field_type_names_the_type_and_the_field() {
    // The milestone's promise is that a rejection names the Lean constant
    // responsible; "a list field would need owned storage" alone did not.
    let ir = r#"
(module M
  (type "M.Rec" (ctor "M.Rec.mk" (xs (List Nat)))))
"#;
    assert_eq!(
        generate_err(ir),
        Error::UnsupportedFieldType(
            "`M.Rec.xs`: a list field would need owned storage".to_string()
        )
    );

    let ir = r#"
(module M
  (type "M.Rec" (ctor "M.Rec.mk" (xs (Vec Nat)))))
"#;
    assert_eq!(
        generate_err(ir),
        Error::UnsupportedFieldType(
            "`M.Rec.xs`: a vector field would need heap storage".to_string()
        )
    );
}

#[test]
fn test_generate_zero_param_golden_def() {
    // The shape prod-export uses for goldens.ir entries.
    let ir = r#"
(module UorAtlas.Goldens
  (def golden_stride_canonical () Nat 24)

  (def golden_classDecode_43_canonical () (Tuple Nat (Tuple Nat Nat))
    (ctor "Prod.mk" 1 (ctor "Prod.mk" 2 3)))
)
"#;
    let out = generate(ir);
    assert_eq!(
        out,
        "pub fn golden_stride_canonical() -> u64 {\n    24\n}\n\npub fn golden_classDecode_43_canonical() -> (u64, (u64, u64)) {\n    (1, (2, 3))\n}\n\n"
    );
}

#[test]
fn test_generate_jp_jmp_inlined() {
    let ir = r#"
(module M
  (def f ((x Nat)) Nat
    (let g (jp g (a) (add a 1)) (jmp g x)))
)
"#;
    let out = generate(ir);
    assert_eq!(
        out,
        "pub fn f(x: u64) -> Result<u64, crate::ComputeError> {\n    Ok({ let g = /* jp \"g\" inlined at its jump site */ (); { let a = x; ((a) as u64).checked_add(1).ok_or(crate::ComputeError::AddOverflow)? } })\n}\n\n"
    );
}

#[test]
fn test_cyclic_join_point_is_rejected() {
    // This used to emit a `loop { /* manual port required */ }` skeleton at
    // exit 0. That skeleton never bound the join point's parameter and left
    // `()` where the arm needed a value, so it was Rust that did not compile —
    // the same silently-broken output this crate rejects everywhere else.
    let ir = r#"
(module M
  (def f ((x Nat)) Nat
    (jp loop (i) (if (lt i 10) (jmp loop (add i 1)) i)))
)
"#;
    assert_eq!(
        generate_err(ir),
        Error::UnsupportedJoinPoint("loop".to_string())
    );
}

#[test]
fn test_multi_caller_join_point_is_rejected() {
    // Two `jmp` sites for one `jp`. LCNF produces this from something as
    // ordinary as a `match` whose arms both feed a shared continuation, so it
    // is not an exotic corner — see `Conformance.c_ctor_body_only`.
    let ir = r#"
(module M
  (def f ((c Nat) (x Nat)) Nat
    (let g (jp g (a) (add a 1))
      (if (lt c 1) (jmp g x) (jmp g c))))
)
"#;
    assert_eq!(
        generate_err(ir),
        Error::UnsupportedJoinPoint("g".to_string())
    );
}

#[test]
fn test_join_point_with_no_callers_still_renders() {
    // The two supported forms are unaffected: no callers renders the body in
    // place, and exactly one caller inlines (covered above).
    let ir = r#"
(module M
  (def f ((x Nat)) Nat
    (jp g (a) x))
)
"#;
    assert!(generate(ir).contains("no jump sites"));
}

#[test]
fn test_generate_pow() {
    let ir = r#"
(module M
  (def belt ((i Nat)) Nat
    (pow 2 (sub i 1)))
)
"#;
    let out = generate(ir);
    assert_eq!(
        out,
        "pub fn belt(i: u64) -> Result<u64, crate::ComputeError> {\n    Ok(((2) as u64).checked_pow(u32::try_from(((i) as u64).saturating_sub(1)).map_err(|_| crate::ComputeError::PowExponentTooLarge)?).ok_or(crate::ComputeError::PowOverflow)?)\n}\n\n"
    );
}

#[test]
fn test_generate_shr() {
    // `Nat.shiftRight` is total and infallible (Lean's `Nat` is unbounded, so
    // `a >>> b = 0` once `b >= 64` and `a` fits `u64`): no `ComputeError`
    // variant, no `?`, just `checked_shr(..).unwrap_or(0)`.
    let ir = r#"
(module M
  (def half ((n Nat) (k Nat)) Nat
    (shr n k))
)
"#;
    let out = generate(ir);
    assert_eq!(
        out,
        "pub fn half(n: u64, k: u64) -> u64 {\n    ((n) as u64).checked_shr(u32::try_from(k).unwrap_or(u32::MAX)).unwrap_or(0)\n}\n\n"
    );
}

#[test]
fn test_generate_decide_unwrapped_comparison_is_a_plain_bool() {
    // Mirrors the IR shape `lean/Prod/Lower.lean`'s `decideOf?` produces for
    // `Conformance.c_bool` (`a < b : Bool`, not consumed by an `if`): a
    // decidable comparison bound directly to a `Bool`-typed `let` renders as
    // a plain Rust comparison, never round-tripping through `Decidable`.
    let ir = r#"
(module M
  (def c_bool ((a Nat) (b Nat)) Bool
    (let x (lt a b) x))
)
"#;
    let out = generate(ir);
    assert_eq!(
        out,
        "pub fn c_bool(a: u64, b: u64) -> bool {\n    { let x = (a < b); x }\n}\n\n"
    );
}

#[test]
fn test_generate_nat_arithmetic_policy_never_panics() {
    let ir = r#"
(module M
  (def add ((x Nat) (y Nat)) Nat (add x y))
  (def sub ((x Nat) (y Nat)) Nat (sub x y))
  (def div ((x Nat) (y Nat)) Nat (div x y))
  (def modu ((x Nat) (y Nat)) Nat (mod x y))
  (def shl ((x Nat) (y Nat)) Nat (shl x y))
  (def pow ((x Nat) (y Nat)) Nat (pow x y))
)
"#;
    let out = generate(ir);
    assert!(out.contains("checked_add(y).ok_or(crate::ComputeError::AddOverflow)?"));
    assert!(out.contains("saturating_sub(y)"));
    assert!(out.contains("if (y) == 0 { 0 } else { (x) / (y) }"));
    assert!(out.contains("if (y) == 0 { 0 } else { (x) % (y) }"));
    assert!(out.contains("checked_shl(u32::try_from(y).map_err(|_| crate::ComputeError::ShiftExponentTooLarge)?).ok_or(crate::ComputeError::ShiftOverflow)?"));
    assert!(out.contains("checked_pow(u32::try_from(y).map_err(|_| crate::ComputeError::PowExponentTooLarge)?).ok_or(crate::ComputeError::PowOverflow)?"));
    // The whole point: no panicking exit remains in the arithmetic lowering.
    assert!(!out.contains(".expect("));
    assert!(!out.contains(".unwrap("));
    assert!(!out.contains("panic!"));
}

#[test]
fn test_generate_unreachable() {
    let ir = "(module M (def f ((x Nat)) Nat (unreachable)))";
    let out = generate(ir);
    assert_eq!(out, "pub fn f(x: u64) -> u64 {\n    unreachable!()\n}\n\n");
}

#[test]
fn test_param_out_of_bounds_is_an_error() {
    let ir = "(module M (def f ((x Nat)) Nat (param 5)))";
    let (_, module) = parse_module(ir).unwrap();
    assert_eq!(generate_module(&module), Err(Error::ParamOutOfBounds(5)));
}

#[test]
fn test_extern_call_is_rejected_not_emitted() {
    // Before this, an untagged callee still rendered as a plain Rust call to a
    // function nobody defined, and the failure surfaced far away in rustc.
    let ir = r#"(module M (def f ((x Nat)) Nat (extern "Foo.helper" x)))"#;
    assert_eq!(
        generate_err(ir),
        Error::UnresolvedCall("Foo.helper".to_string())
    );
}

#[test]
fn test_generate_struct_from_single_ctor_type() {
    let ir = r#"
(module M
  (type "UorAtlas.Instance"
    (ctor "UorAtlas.Instance.mk" (q Nat) (T Nat) (O Nat)))
  (def stride ((i (named "UorAtlas.Instance"))) Nat 0)
)
"#;
    let out = generate(ir);
    assert!(out.contains(
        "#[derive(Debug, Clone, Copy, PartialEq, Eq)]\npub struct Instance {\n    pub q: u64,\n    pub T: u64,\n    pub O: u64,\n}\n"
    ));
    assert!(out.contains("pub fn stride(i: crate::Instance) -> u64 {"));
}

#[test]
fn test_generate_enum_from_multi_ctor_type() {
    let ir = r#"
(module M
  (type "M.Shape"
    (ctor "M.Shape.circle" (radius Nat))
    (ctor "M.Shape.rect" (w Nat) (h Nat)))
)
"#;
    let out = generate(ir);
    assert!(out.contains(
        "#[derive(Debug, Clone, Copy, PartialEq, Eq)]\npub enum Shape {\n    circle { radius: u64 },\n    rect { w: u64, h: u64 },\n}\n"
    ));
}

#[test]
fn test_generate_fieldless_ctor_renders_unit_variant() {
    let ir = r#"
(module M
  (type "M.Flag" (ctor "M.Flag.off") (ctor "M.Flag.on")))
"#;
    let out = generate(ir);
    assert!(out.contains("pub enum Flag {\n    off,\n    on,\n}\n"));
}

#[test]
fn test_rust_keyword_field_names_are_raw_escaped() {
    // A Lean field named `type` or `fn` is legal Lean and illegal Rust.
    let ir = r#"
(module M
  (type "M.Rec" (ctor "M.Rec.mk" (type Nat) (fn Nat))))
"#;
    let out = generate(ir);
    assert!(out.contains("pub r#type: u64"));
    assert!(out.contains("pub r#fn: u64"));
}

#[test]
fn test_recursive_type_is_rejected() {
    let ir = r#"
(module M
  (type "M.Tree"
    (ctor "M.Tree.leaf")
    (ctor "M.Tree.node" (left (named "M.Tree")) (right (named "M.Tree")))))
"#;
    assert_eq!(generate_err(ir), Error::RecursiveType("M.Tree".to_string()));
}

#[test]
fn test_duplicate_last_component_is_rejected() {
    let ir = r#"
(module M
  (type "A.Thing" (ctor "A.Thing.mk" (x Nat)))
  (type "B.Thing" (ctor "B.Thing.mk" (y Nat))))
"#;
    assert_eq!(
        generate_err(ir),
        Error::DuplicateTypeName("Thing".to_string())
    );
}

#[test]
fn test_polymorphic_type_is_rejected_with_its_reason() {
    // The exporter cannot describe a parameterised inductive, so it declares
    // the type as unsupported rather than omitting it — that turns a generic
    // "unknown type" into a rejection that names monomorphization.
    let ir = r#"(module M (type "M.Box" (unsupported "type parameters")))"#;
    assert_eq!(
        generate_err(ir),
        Error::PolymorphicType("M.Box".to_string())
    );
}

#[test]
fn test_undeclared_named_type_in_a_signature_is_rejected() {
    let ir = r#"(module M (def f ((x (named "M.Nope"))) Nat 0))"#;
    assert!(matches!(generate_err(ir), Error::OpaqueType(_)));
}

#[test]
fn test_undeclared_named_type_in_a_return_position_is_rejected() {
    let ir = r#"(module M (def f ((x Nat)) (named "M.Nope") x))"#;
    assert!(matches!(generate_err(ir), Error::OpaqueType(_)));
}

#[test]
fn test_generate_named_struct_construction() {
    let ir = r#"
(module M
  (type "UorAtlas.Instance"
    (ctor "UorAtlas.Instance.mk" (q Nat) (T Nat) (O Nat)))
  (def mk ((a Nat) (b Nat) (c Nat)) (named "UorAtlas.Instance")
    (ctor "UorAtlas.Instance.mk" a b c)))
"#;
    let out = generate(ir);
    assert!(out.contains("crate::Instance { q: a, T: b, O: c }"));
}

#[test]
fn test_generate_enum_construction_and_patterns() {
    let ir = r#"
(module M
  (type "M.Shape"
    (ctor "M.Shape.circle" (radius Nat))
    (ctor "M.Shape.rect" (w Nat) (h Nat)))
  (def area ((s (named "M.Shape"))) Nat
    (cases s
      (alt "M.Shape.circle" (r) r)
      (alt "M.Shape.rect" (w h) (mul w h))))
  (def unit ((r Nat)) (named "M.Shape") (ctor "M.Shape.circle" r)))
"#;
    let out = generate(ir);
    assert!(out.contains("crate::Shape::circle { radius: r } => r,"));
    assert!(out.contains("crate::Shape::rect { w: w, h: h } =>"));
    assert!(out.contains("crate::Shape::circle { radius: r }"));
}

#[test]
fn test_ctor_arity_mismatch_is_an_error() {
    let ir = r#"
(module M
  (type "M.Pair" (ctor "M.Pair.mk" (a Nat) (b Nat)))
  (def f ((x Nat)) (named "M.Pair") (ctor "M.Pair.mk" x)))
"#;
    assert!(matches!(generate_err(ir), Error::UnsupportedFieldType(_)));
}

#[test]
fn test_cases_alt_arity_mismatch_on_a_declared_ctor_is_an_error() {
    // The alt names a declared two-field constructor but binds only one
    // binder. Falling through to the positional fallback would render
    // `M.Pair.mk(x) => ...` — a dotted name as a Rust path with a
    // positional pattern, which does not compile. This must be rejected,
    // symmetric with the construction-side arity check.
    let ir = r#"
(module M
  (type "M.Pair" (ctor "M.Pair.mk" (a Nat) (b Nat)))
  (def f ((p (named "M.Pair"))) Nat
    (cases p
      (alt "M.Pair.mk" (x) x))))
"#;
    assert!(matches!(generate_err(ir), Error::UnsupportedFieldType(_)));
}

#[test]
fn test_cases_alt_on_an_undeclared_ctor_still_falls_through() {
    // Regression guard: an alt naming a constructor that is NOT in this
    // module's type table (not even the same arity check applies) must keep
    // the existing positional-fallback rendering untouched.
    let ir = r#"
(module M
  (def f ((x Nat)) Nat
    (cases x
      (alt "Foo.Bar" (a b) (add a b))
      (default 0))))
"#;
    let out = generate(ir);
    assert!(out.contains("Foo.Bar(a, b) => "));
}

#[test]
fn test_opaque_type_is_rejected_not_injected() {
    // Previously rendered the raw Lean name as a Rust type, which exploded
    // inside syn::parse_str with an error pointing nowhere near the cause.
    let ir = r#"(module M (def f ((x (opaque "Foo.Bar"))) Nat 0))"#;
    assert_eq!(generate_err(ir), Error::OpaqueType("Foo.Bar".to_string()));
}

#[test]
fn test_projection_of_an_undeclared_field_is_rejected() {
    // A projection must name a field its type actually declares. Without this,
    // a declaration and a projection can disagree inside one IR file and still
    // compile, as long as something else supplies a type with the other
    // spelling.
    let ir = r#"
(module M
  (type "M.Rec" (ctor "M.Rec.mk" (alpha Nat)))
  (def f ((r (named "M.Rec"))) Nat (proj "M.Rec" "beta" r)))
"#;
    assert_eq!(
        generate_err(ir),
        Error::UnknownField("M.Rec".to_string(), "beta".to_string())
    );
}

#[test]
fn test_projection_of_a_declared_field_still_renders() {
    let ir = r#"
(module M
  (type "M.Rec" (ctor "M.Rec.mk" (alpha Nat)))
  (def f ((r (named "M.Rec"))) Nat (proj "M.Rec" "alpha" r)))
"#;
    assert!(generate(ir).contains("(r).alpha"));
}
