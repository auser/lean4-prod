use super::*;
use alloc::vec;
use prod_ir::parser::parse_module;
extern crate std;

fn generate(ir: &str) -> String {
    let (_, module) = parse_module(ir).unwrap();
    generate_module(&module).unwrap()
}

fn generate_err(ir: &str) -> Error {
    let (_, module) = parse_module(ir).unwrap();
    generate_module(&module).unwrap_err()
}

#[test]
fn unused_eager_join_arguments_still_reject_unsupported_ir() {
    for (argument, expected) in [
        (
            r#"(extern "Missing.fn")"#,
            Error::UnresolvedCall(String::from("Missing.fn")),
        ),
        (
            r#"(opaque "unsupported")"#,
            Error::OpaqueExpr(String::from("unsupported")),
        ),
        (
            r#"(let hidden (extern "Missing.fn") 1)"#,
            Error::UnresolvedCall(String::from("Missing.fn")),
        ),
        (
            r#"(if true 0 (opaque "unsupported"))"#,
            Error::OpaqueExpr(String::from("unsupported")),
        ),
    ] {
        let input = format!(
            "(module M (def probe () Nat (let continuation (jp continuation (unused) 7) (jmp continuation {argument}))))"
        );
        let (remaining, module) = parse_module(&input).unwrap();
        assert!(remaining.is_empty());
        assert_eq!(generate_module(&module), Err(expected), "{argument}");
    }
}

#[test]
fn computed_eager_list_arguments_do_not_enter_lazy_builders() {
    for argument in [
        "(ctor \"List.cons\" (add input 1) (ctor \"List.nil\"))",
        "(if true (ctor \"List.cons\" (add input 1) (ctor \"List.nil\")) (ctor \"List.nil\"))",
        "(let list (ctor \"List.cons\" (add input 1) (ctor \"List.nil\")) list)",
        "(cases true (alt \"Bool.true\" () (ctor \"List.cons\" (add input 1) (ctor \"List.nil\"))) (alt \"Bool.false\" () (ctor \"List.nil\")))",
    ] {
        let input = format!(
            "(module M (def probe ((input Nat)) (List Nat) (let g (jp g (unused) (ctor \"List.nil\")) (jmp g {argument}))))"
        );
        assert!(matches!(generate_err(&input), Error::UnsupportedList(_)), "{argument}");
    }
    // The second expansion renames the repeated formal. Its eager identity
    // must survive that normalization instead of becoming an ordinary lazy let.
    assert!(matches!(
        generate_err(
            r#"(module M (def probe ((input Nat)) (List Nat)
          (let g (jp g (unused) (ctor "List.nil"))
            (if (eq input 0)
              (jmp g (ctor "List.cons" 1 (ctor "List.nil")))
              (jmp g (ctor "List.cons" (add input 1) (ctor "List.nil")))))))"#
        ),
        Error::UnsupportedList(_)
    ));
    assert!(matches!(
        generate_err(
            r#"(module M (def probe () (List Nat)
          (let g (jp g (unused) (ctor "List.nil"))
            (jmp g (ctor "List.cons" (unreachable) (ctor "List.nil"))))))"#
        ),
        Error::UnsupportedList(_)
    ));
    assert_eq!(
        generate_err(
            r#"(module M (def probe ((input Nat)) (List Nat)
          (let g (jp g (unused) (ctor "List.nil"))
            (jmp g (ctor "List.cons" (extern "Missing.fn") (ctor "List.nil"))))))"#
        ),
        Error::UnresolvedCall(String::from("Missing.fn"))
    );
    // Scalar continuations inside a constant list head remain supported.
    let source = generate(
        r#"(module M (def probe () (List Nat)
      (ctor "List.cons" (let g (jp g (value) value) (jmp g 7)) (ctor "List.nil"))))"#,
    );
    assert!(source.contains("&'static [u64]"));
}

#[test]
fn eager_list_pattern_results_cannot_allocate_behind_builder_guards() {
    assert!(matches!(
        generate_err(
            r#"(module M (def probe ((input (List Nat))) (List Nat)
          (let g (jp g (unused) (ctor "List.nil")) (jmp g (append input input)))))"#
        ),
        Error::UnsupportedList(_)
    ));
    for argument in [
        r#"(cases (ctor "Wrapped.mk" (ctor "List.cons" (add input 1) (ctor "List.nil")))
          (alt "Wrapped.mk" (items) items))"#,
        r#"(cases (ctor "Option.some" (ctor "List.cons" (add input 1) (ctor "List.nil")))
          (alt "Option.some" (items) items) (alt "Option.none" () (ctor "List.nil")))"#,
        r#"(cases (ctor "List.cons" input (ctor "List.nil"))
          (alt "List.cons" (head tail) tail) (alt "List.nil" () (ctor "List.nil")))"#,
    ] {
        let input = format!(
            r#"(module M
          (type "Wrapped" (ctor "Wrapped.mk" (items (List Nat))))
          (def probe ((input Nat)) (List Nat)
            (let g (jp g (unused) (ctor "List.nil")) (jmp g {argument}))))"#
        );
        assert!(
            matches!(generate_err(&input), Error::UnsupportedList(_)),
            "{argument}"
        );
    }
    let source = generate(
        r#"(module M
      (type "Wrapped" (ctor "Wrapped.mk" (value Nat)))
      (def probe ((input Nat)) (List Nat)
        (let g (jp g (unused) (ctor "List.nil"))
          (jmp g (cases (ctor "Wrapped.mk" input)
            (alt "Wrapped.mk" (value) value))))))"#,
    );
    assert!(source.contains("match"));
    assert!(!source.contains("alloc::vec"));
}

fn package_file<'a>(package: &'a GeneratedPackage, path: &str) -> &'a [u8] {
    &package
        .files
        .iter()
        .find(|file| file.path == path)
        .unwrap_or_else(|| panic!("missing generated package file {path}"))
        .bytes
}

#[test]
fn test_generate_publishable_cargo_package_is_closed_and_deterministic() {
    let (_, module) = parse_module(
        "(module M (def add64 ((a Int64) (b Int64)) (Option Int64) (checked-add a b)))",
    )
    .unwrap();
    let spec = CargoPackageSpec {
        name: "generated-core".to_string(),
        version: "0.1.0".to_string(),
        description: "Generated core".to_string(),
        repository: "https://example.invalid/core".to_string(),
        homepage: "https://example.invalid/".to_string(),
        readme: "# Generated core\n".to_string(),
        license_mit: "MIT\n".to_string(),
        license_apache: "Apache-2.0\n".to_string(),
        input_sha256: "00".repeat(32),
        dependencies: vec![CargoDependency {
            name: "generated-runtime".to_string(),
            version: "0.1.0".to_string(),
            checksum: "11".repeat(32),
            default_features: false,
            features: Vec::new(),
        }],
    };
    let first = generate_cargo_package(&module, &spec).unwrap();
    let second = generate_cargo_package(&module, &spec).unwrap();
    assert_eq!(first, second);
    assert_eq!(
        first
            .files
            .iter()
            .map(|file| file.path.as_str())
            .collect::<Vec<_>>(),
        vec![
            "Cargo.lock",
            "Cargo.toml",
            "LICENSE-APACHE",
            "LICENSE-MIT",
            "README.md",
            "generation-manifest.json",
            "src/lib.rs",
        ]
    );
    let manifest = core::str::from_utf8(package_file(&first, "Cargo.toml")).unwrap();
    assert!(manifest.contains("default = [\"std\"]"));
    assert!(manifest.contains("generated-runtime = { version = \"=0.1.0\""));
    assert!(!manifest.contains("{ path ="));
    assert!(!manifest.contains("git ="));
    let generation =
        core::str::from_utf8(package_file(&first, "generation-manifest.json")).unwrap();
    assert!(generation.contains(
        "\"dependencies\":[{\"checksum\":\"1111111111111111111111111111111111111111111111111111111111111111\",\"default_features\":false,\"features\":[],\"name\":\"generated-runtime\",\"version\":\"0.1.0\"}]"
    ));
    // The package boundary owns canonical source termination, including empty
    // modules. Its manifest must hash the final bytes, not the pre-normalized
    // module output. Caller-owned license/readme bytes must remain untouched.
    for ir in [
        "(module Empty)",
        "(module Single (def value () Bool true))",
        "(module Multiple (def first () Bool true) (def second () Bool false))",
    ] {
        use sha2::{Digest, Sha256};

        let (_, module) = parse_module(ir).unwrap();
        let package = generate_cargo_package(&module, &spec).unwrap();
        let source = package_file(&package, "src/lib.rs");
        assert!(source.ends_with(b"}\n"), "{ir}: noncanonical source EOF");
        let digest = format!("{:x}", Sha256::digest(source));
        let generation =
            core::str::from_utf8(package_file(&package, "generation-manifest.json")).unwrap();
        assert!(generation.contains(&format!(
            "{{\"path\":\"src/lib.rs\",\"sha256\":\"{digest}\"}}"
        )));
        assert_eq!(
            package_file(&package, "LICENSE-MIT"),
            spec.license_mit.as_bytes()
        );
        assert_eq!(
            package_file(&package, "LICENSE-APACHE"),
            spec.license_apache.as_bytes()
        );
        assert_eq!(package_file(&package, "README.md"), spec.readme.as_bytes());
    }
}

#[test]
fn test_mathematical_int_is_never_silently_lowered_to_i64() {
    let ir = "(module M (def identity ((value Int)) Int value))";
    assert_eq!(generate_err(ir), Error::UnboundedInt);
}

#[test]
fn test_generate_core_wasm_package_has_exact_generic_abi_contract() {
    let (_, module) =
        parse_module(r#"(module M (def echo ((input (List UInt8))) (List UInt8) input))"#).unwrap();
    let spec = CoreWasmSpec {
        crate_name: "generated-guest".to_string(),
        entry: "echo".to_string(),
        export_name: "holo_run".to_string(),
        input_allocation_cap: 65_536,
        output_allocation_cap: 27,
        maximum_pages: 4,
        input_ir_sha256: "11".repeat(32),
    };
    let first = generate_core_wasm_package(&module, &spec).unwrap();
    assert_eq!(first, generate_core_wasm_package(&module, &spec).unwrap());
    let source = core::str::from_utf8(package_file(&first, "src/lib.rs")).unwrap();
    assert!(source.contains("pub extern \"C\" fn holo_alloc(size: i32) -> i32"));
    assert!(source.contains("pub extern \"C\" fn holo_run(input_ptr: i32, input_len: i32) -> i64"));
    assert!(source.contains("compare_exchange_weak"));
    assert!(source.contains("if input_ptr < output_end && output_ptr < input_end"));
    assert!(source.contains("panic(_: &core::panic::PanicInfo<'_>) -> ! { trap() }"));
    let config = core::str::from_utf8(package_file(&first, ".cargo/config.toml")).unwrap();
    assert!(config.contains("--export-memory"));
    assert!(config.contains("--max-memory=262144"));
}

#[test]
fn test_core_wasm_rejects_non_byte_dispatchers_and_invalid_bounds() {
    let (_, module) = parse_module("(module M (def scalar ((input UInt8)) UInt8 input))").unwrap();
    let spec = CoreWasmSpec {
        crate_name: "generated-guest".to_string(),
        entry: "scalar".to_string(),
        export_name: "holo_run".to_string(),
        input_allocation_cap: 65_536,
        output_allocation_cap: 27,
        maximum_pages: 4,
        input_ir_sha256: "11".repeat(32),
    };
    assert!(matches!(
        generate_core_wasm_package(&module, &spec),
        Err(Error::UnsupportedList(_))
    ));
    let mut invalid = spec;
    invalid.maximum_pages = 0;
    assert!(generate_core_wasm_package(&module, &invalid).is_err());
}

#[test]
fn test_core_wasm_bytes_entry_uses_module_fallibility() {
    let spec = CoreWasmSpec {
        crate_name: "bytes-guest".to_string(),
        entry: "entry".to_string(),
        export_name: "holo_run".to_string(),
        input_allocation_cap: 128,
        output_allocation_cap: 64,
        maximum_pages: 4,
        input_ir_sha256: "11".repeat(32),
    };
    for (ir, fallible) in [
        (
            "(module Infallible (def entry ((input Bytes)) Bytes input)
              (def unrelated () Nat (add 18446744073709551615 1)))",
            false,
        ),
        (
            "(module Direct (def entry ((input Bytes)) Bytes
              (let checked (add 18446744073709551615 1) input)))",
            true,
        ),
        (
            "(module Transitive
              (def entry ((input Bytes)) Bytes (call middle input))
              (def middle ((input Bytes)) Bytes (call leaf input))
              (def leaf ((input Bytes)) Bytes
                (let checked (add 18446744073709551615 1) input)))",
            true,
        ),
    ] {
        let (_, module) = parse_module(ir).unwrap();
        let package = generate_core_wasm_package(&module, &spec).unwrap();
        let source = core::str::from_utf8(package_file(&package, "src/lib.rs")).unwrap();
        let suffix = if fallible {
            ".unwrap_or_else(|_| trap())"
        } else {
            ""
        };
        assert!(
            source.contains(&format!(
                "let generated_output = entry(input.to_vec()){suffix};"
            )),
            "wrong Bytes ABI adapter for {ir}"
        );
        assert_eq!(package, generate_core_wasm_package(&module, &spec).unwrap());
    }
}

fn view_fixture() -> (EvaluatedViewV1, BrowserAdapterBinding) {
    (
        EvaluatedViewV1 {
            title: "Calculator <safe>".to_string(),
            heading: "Calculator".to_string(),
            left_label: "Left".to_string(),
            right_label: "Right".to_string(),
            operation_label: "Operation".to_string(),
            submit_label: "Calculate".to_string(),
            input_error: "Enter a signed 64-bit integer.".to_string(),
            division_by_zero_error: "Division by zero.".to_string(),
            overflow_error: "Arithmetic overflow.".to_string(),
            operations: vec![
                ViewOperation {
                    label: "Add".to_string(),
                    request_name: "add".to_string(),
                    rust_variant: "Add".to_string(),
                    discriminant: 0,
                },
                ViewOperation {
                    label: "Divide".to_string(),
                    request_name: "divide".to_string(),
                    rust_variant: "Divide".to_string(),
                    discriminant: 3,
                },
            ],
            initial_operation: 0,
            model_id: "22".repeat(32),
            view_model_id: "33".repeat(32),
            generated_core_sha256: "44".repeat(32),
        },
        BrowserAdapterBinding {
            package_name: "generated-browser-adapter".to_string(),
            package_version: "0.1.0".to_string(),
            core_crate_name: "prism-calculator".to_string(),
            core_crate_version: "0.1.0".to_string(),
            core_operation_type: "Operation".to_string(),
            core_error_type: "CalculatorError".to_string(),
            core_function: "calculate".to_string(),
        },
    )
}

#[test]
fn test_view_v1_projects_both_transports_without_raw_content() {
    let (view, binding) = view_fixture();
    let generated = generate_view_v1(&view, &binding).unwrap();
    assert_eq!(generated, generate_view_v1(&view, &binding).unwrap());
    assert!(generated
        .hologram_bundle
        .bytes
        .starts_with(b"HOLOVIEW\0\x01"));
    assert_eq!(&generated.hologram_bundle.bytes[14..24], b"index.html");
    assert_eq!(
        generated
            .hologram_assets
            .iter()
            .map(|file| file.path.as_str())
            .collect::<Vec<_>>(),
        vec!["app.css", "app.js", "index.html"]
    );
    let index = core::str::from_utf8(
        &generated
            .hologram_assets
            .iter()
            .find(|file| file.path == "index.html")
            .unwrap()
            .bytes,
    )
    .unwrap();
    assert!(index.contains("<title>Calculator &lt;safe&gt;</title>"));
    assert!(
        index.contains("<div class=\"field\"><label for=\"left\">Left</label><input id=\"left\"")
    );
    assert!(index.contains(
        "<div class=\"field\"><label for=\"operation\">Operation</label><select id=\"operation\""
    ));
    assert!(index
        .contains("<div class=\"field\"><label for=\"right\">Right</label><input id=\"right\""));
    let stylesheet = core::str::from_utf8(
        &generated
            .hologram_assets
            .iter()
            .find(|file| file.path == "app.css")
            .unwrap()
            .bytes,
    )
    .unwrap();
    assert!(stylesheet.contains(".field{display:grid;gap:.35rem;min-width:0}"));
    assert!(stylesheet.contains("input,select,button{width:100%"));
    let intent = core::str::from_utf8(
        &generated
            .hologram_assets
            .iter()
            .find(|file| file.path == "app.js")
            .unwrap()
            .bytes,
    )
    .unwrap();
    assert!(intent.contains("'/_hologram/intent'"));
    assert!(intent.contains("name:'application.invoke'"));
    let browser = core::str::from_utf8(
        &generated
            .browser_assets
            .iter()
            .find(|file| file.path == "app.js")
            .unwrap()
            .bytes,
    )
    .unwrap();
    assert!(browser.contains("calculate(selected,a,b)"));
    assert!(browser.contains("const MIN=-9223372036854775808n"));
    assert!(browser.contains("/^(?:0|-[1-9][0-9]*|[1-9][0-9]*)$/"));
    assert!(!browser.contains("value.replace"));
    assert!(!browser.contains("parsed.toString()"));
    let cargo =
        core::str::from_utf8(package_file(&generated.browser_adapter, "Cargo.toml")).unwrap();
    assert!(cargo.contains("prism-calculator = \"=0.1.0\""));
    assert!(!cargo.contains("path ="));
    assert!(!cargo.contains("git ="));
}

#[test]
fn test_view_v1_never_uses_native_form_submission() {
    let (view, binding) = view_fixture();
    let generated = generate_view_v1(&view, &binding).unwrap();
    for assets in [&generated.browser_assets, &generated.hologram_assets] {
        let index = core::str::from_utf8(&assets[2].bytes).unwrap();
        assert!(index.find("form-action 'none'").unwrap() < index.find("<form").unwrap());
        for name in ["left", "right", "operation"] {
            assert!(!index.contains(&format!("name=\"{name}\"")));
        }
        assert!(index.contains("id=\"submit\" type=\"submit\" disabled"));
        assert!(index.contains(&format!(">{}</output>", view.input_error)));
        let script = core::str::from_utf8(&assets[1].bytes).unwrap();
        assert!(
            script.find("event.preventDefault()").unwrap()
                < script.find("submit.disabled=false").unwrap()
        );
    }
    let script = core::str::from_utf8(&generated.browser_assets[1].bytes).unwrap();
    assert!(script.contains("import('./prism_calculator.js')"));
    assert!(!script.starts_with("import "));
    assert!(script.contains(".catch(()=>{show(INPUT_ERROR);return null;})"));
}

#[test]
fn test_view_v1_rejects_duplicate_or_injectable_operations() {
    let (mut view, binding) = view_fixture();
    view.operations[1].rust_variant = "Divide; panic!()".to_string();
    assert!(generate_view_v1(&view, &binding).is_err());
    let (mut view, binding) = view_fixture();
    view.operations[1].discriminant = 0;
    assert!(generate_view_v1(&view, &binding).is_err());
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
        "#[derive(Debug, Clone, Copy, PartialEq, Eq)]\npub struct Instance {\n    pub q: u64,\n    pub T: u64,\n    pub O: u64,\n}\n\npub fn classIndex(h2: u64, d: u64, l: u64, inst: crate::Instance) -> Result<u64, crate::ComputeError> {\n    Ok(core::convert::identity::<u64>(core::convert::identity::<u64>(stride(inst)).checked_mul(h2).ok_or(crate::ComputeError::MulOverflow)?).checked_add(core::convert::identity::<u64>(core::convert::identity::<u64>((inst).O).checked_mul(d).ok_or(crate::ComputeError::MulOverflow)?).checked_add(l).ok_or(crate::ComputeError::AddOverflow)?).ok_or(crate::ComputeError::AddOverflow)?)\n}\n\npub fn belt(inst: crate::Instance) -> Result<u64, crate::ComputeError> {\n    Ok(core::convert::identity::<u64>(class_count(inst)).checked_mul(core::convert::identity::<u64>(1).checked_shl(u32::try_from(core::convert::identity::<u64>(core::convert::identity::<u64>((inst).O).saturating_sub(1))).map_err(|_| crate::ComputeError::ShiftExponentTooLarge)?).ok_or(crate::ComputeError::ShiftOverflow)?).ok_or(crate::ComputeError::MulOverflow)?)\n}\n\n"
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
        "pub fn digitSum(xs: &[u64]) -> Result<u64, crate::ComputeError> {\n    Ok(match xs {\n        [] => 0,\n        [h, t @ ..] => { let h = h.clone(); core::convert::identity::<u64>(h).checked_add(digitSum(&(t))?).ok_or(crate::ComputeError::AddOverflow)? },\n    })\n}\n\npub fn digits(n: u64, output: &mut [u64]) -> Result<usize, crate::ComputeError> {\n    if (n < 8) { match (output).split_first_mut() { None => Err(crate::ComputeError::OutputTooSmall), Some((__head0, __rest0)) => { *__head0 = n; let __len0 = Ok::<usize, crate::ComputeError>(0)?; Ok(__len0 + 1) } } } else { match (output).split_first_mut() { None => Err(crate::ComputeError::OutputTooSmall), Some((__head0, __rest0)) => { *__head0 = match (core::convert::identity::<u64>(n), core::convert::identity::<u64>(8)) { (__left, 0) => __left, (__left, __right) => __left % __right }; let __len0 = digits(match (core::convert::identity::<u64>(n), core::convert::identity::<u64>(8)) { (__left, 0) => 0, (__left, __right) => __left / __right }, __rest0)?; Ok(__len0 + 1) } } }\n}\n\n"
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
        "#[derive(Debug, Clone, Copy, PartialEq, Eq)]\npub struct Instance {\n    pub q: u64,\n    pub T: u64,\n    pub O: u64,\n}\n\npub fn digits(fuel: u64, n: u64, i: crate::Instance, output: &mut [u64]) -> Result<usize, crate::ComputeError> {\n    match fuel {\n        0 => Ok::<usize, crate::ComputeError>(0),\n        _ => { let n_25 = (fuel).saturating_sub(1); { let _x_47 = (i).O; if (n < _x_47) { match (output).split_first_mut() { None => Err(crate::ComputeError::OutputTooSmall), Some((__head0, __rest0)) => { *__head0 = n; let __len0 = Ok::<usize, crate::ComputeError>(0)?; Ok(__len0 + 1) } } } else { { let _x_50 = match (core::convert::identity::<u64>(n), core::convert::identity::<u64>(_x_47)) { (__left, 0) => __left, (__left, __right) => __left % __right }; { let _x_51 = match (core::convert::identity::<u64>(n), core::convert::identity::<u64>(_x_47)) { (__left, 0) => 0, (__left, __right) => __left / __right }; match (output).split_first_mut() { None => Err(crate::ComputeError::OutputTooSmall), Some((__head0, __rest0)) => { *__head0 = _x_50; let __len0 = digits(n_25, _x_51, i, __rest0)?; Ok(__len0 + 1) } } } } } } },\n    }\n}\n\n"
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
    assert!(out.contains("loops(k, core::convert::identity::<u64>(x).checked_add(1)"));
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
fn test_nested_list_type_is_an_explicit_owned_vec() {
    let ir = r#"
(module M
  (def f ((x Nat)) (Option (List Nat)) (ctor "Option.none"))
)
"#;
    assert!(generate(ir).contains("-> Option<alloc::vec::Vec<u64>>"));
}

#[test]
fn test_structure_list_projection_copies_into_the_output_buffer() {
    let ir = r#"
(module M
  (type "M.Relation" (ctor "M.Relation.mk" (bound Nat) (values (List Nat))))
  (type "M.Manifest" (ctor "M.Manifest.mk" (relation (named "M.Relation"))))
  (def values ((self (named "M.Relation"))) (List Nat)
    (let projected (proj "M.Relation" "values" self) projected))
  (def relation ((self (named "M.Manifest"))) (named "M.Relation")
    (let projected (proj "M.Manifest" "relation" self) projected))
)
"#;
    let out = generate(ir);
    assert!(out.contains("pub values: alloc::vec::Vec<u64>"));
    assert!(out.contains("pub fn values(__prod_self: &crate::Relation, output: &mut [u64])"));
    assert!(out.contains("pub fn relation(__prod_self: &crate::Manifest) -> &crate::Relation"));
    assert!(out.contains(
        "if __source.len() > (output).len() { Err(crate::ComputeError::OutputTooSmall) }"
    ));
    assert!(out.contains("clone_from_slice(__source)"));
    assert!(!out.contains("output["));
    assert!(!out.contains("__list"));

    // Compile and execute the generated projection so the undersized-buffer
    // branch is behaviorally pinned: caller-controlled exhaustion is an
    // error and leaves the caller's buffer untouched.
    let directory = std::env::temp_dir().join(std::format!(
        "prod-codegen-list-projection-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).unwrap();
    let source = directory.join("main.rs");
    let executable = directory.join("projection-test");
    std::fs::write(
        &source,
        std::format!(
            r#"extern crate alloc;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComputeError {{ OutputTooSmall }}
{out}
fn main() {{
    let relation_value = Relation {{ bound: 2, values: alloc::vec![10, 20] }};
    let mut exact = [0_u64; 2];
    assert_eq!(values(&relation_value, &mut exact), Ok(2));
    assert_eq!(exact, [10, 20]);
    let mut short = [99_u64; 1];
    assert_eq!(values(&relation_value, &mut short), Err(ComputeError::OutputTooSmall));
    assert_eq!(short, [99]);
    let manifest = Manifest {{ relation: relation_value }};
    assert_eq!(relation(&manifest).bound, 2);
}}
"#
        ),
    )
    .unwrap();
    let compiled = std::process::Command::new("rustc")
        .args(["--edition", "2021"])
        .arg(&source)
        .arg("-o")
        .arg(&executable)
        .status()
        .unwrap();
    assert!(compiled.success());
    assert!(std::process::Command::new(&executable)
        .status()
        .unwrap()
        .success());
    std::fs::remove_dir_all(directory).unwrap();
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
        "pub fn digitCount(fuel: u64, n: u64) -> Result<u64, crate::ComputeError> {\n    Ok(match fuel {\n        0 => 0,\n        _ => { let k = (fuel).saturating_sub(1); if (n < 8) { 1 } else { core::convert::identity::<u64>(1).checked_add(digitCount(k, match (core::convert::identity::<u64>(n), core::convert::identity::<u64>(8)) { (__left, 0) => 0, (__left, __right) => __left / __right })?).ok_or(crate::ComputeError::AddOverflow)? } },\n    })\n}\n\n"
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
fn test_keyword_parameter_is_raw_escaped() {
    let ir = r#"(module M (def identity ((self Nat)) Nat (param 0)))"#;
    assert_eq!(
        generate(ir),
        "pub fn identity(__prod_self: u64) -> u64 {\n    __prod_self\n}\n\n"
    );
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
        "#[derive(Debug, Clone, Copy, PartialEq, Eq)]\npub struct Instance {\n    pub q: u64,\n    pub T: u64,\n    pub O: u64,\n}\n\npub fn stride(i: crate::Instance) -> Result<u64, crate::ComputeError> {\n    Ok({ let _x_4 = (i).T; { let _x_5 = (i).O; { let _x_13 = core::convert::identity::<u64>(_x_4).checked_mul(_x_5).ok_or(crate::ComputeError::MulOverflow)?; _x_13 } } })\n}\n\npub fn classDecode(idx: u64, i: crate::Instance) -> Result<(u64, (u64, u64)), crate::ComputeError> {\n    Ok({ let _x_4 = stride(i)?; { let h2 = match (core::convert::identity::<u64>(idx), core::convert::identity::<u64>(_x_4)) { (__left, 0) => 0, (__left, __right) => __left / __right }; { let rem = match (core::convert::identity::<u64>(idx), core::convert::identity::<u64>(_x_4)) { (__left, 0) => __left, (__left, __right) => __left % __right }; { let _x_10 = (i).O; { let d = match (core::convert::identity::<u64>(rem), core::convert::identity::<u64>(_x_10)) { (__left, 0) => 0, (__left, __right) => __left / __right }; { let l = match (core::convert::identity::<u64>(rem), core::convert::identity::<u64>(_x_10)) { (__left, 0) => __left, (__left, __right) => __left % __right }; { let _x_13 = (d, l); { let _x_14 = (h2, _x_13); _x_14 } } } } } } } })\n}\n\n"
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
        Error::UnboundedInt,
        Error::UnresolvedCall(s()),
        Error::UnknownField(s(), s()),
        Error::UnsupportedJoinPoint(s()),
        Error::JoinExpansionLimit,
        Error::DuplicateBinding(s()),
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
            Error::UnboundedInt => "UnboundedInt",
            Error::UnresolvedCall(_) => "UnresolvedCall",
            Error::UnknownField(..) => "UnknownField",
            Error::UnsupportedJoinPoint(_) => "UnsupportedJoinPoint",
            Error::JoinExpansionLimit => "JoinExpansionLimit",
            Error::DuplicateBinding(_) => "DuplicateBinding",
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
fn test_owned_list_field_is_supported_but_vec_field_names_the_owner() {
    let ir = r#"
(module M
  (type "M.Rec" (ctor "M.Rec.mk" (xs (List Nat)))))
"#;
    assert!(generate(ir).contains("pub xs: alloc::vec::Vec<u64>"));

    // The internal Vec IR remains forbidden and the diagnostic names the
    // declaration and field that introduced it.
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
        "pub fn f(x: u64) -> Result<u64, crate::ComputeError> {\n    Ok({ let a = x; core::convert::identity::<u64>(a).checked_add(1).ok_or(crate::ComputeError::AddOverflow)? })\n}\n\n"
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
fn test_multi_caller_acyclic_join_point_is_inlined_at_every_jump() {
    // LCNF produces this shape when multiple match arms feed one pure
    // continuation. Each branch gets its own parameter binding and checked
    // addition, with no runtime allocation or unbound join parameter.
    let ir = r#"
(module M
  (def f ((c Nat) (x Nat)) Nat
    (let g (jp g (a) (add a 1))
      (if (lt c 1) (jmp g x) (jmp g c))))
)
"#;
    let out = generate(ir);
    assert_eq!(out.matches("let ").count(), 2);
    assert_eq!(out.matches("checked_add(1)").count(), 2);
    assert!(out.contains("let a = x"));
    assert!(out.contains(" = c;"));
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
    assert_eq!(generate(ir), "pub fn f(x: u64) -> u64 {\n    x\n}\n\n");
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
        "pub fn belt(i: u64) -> Result<u64, crate::ComputeError> {\n    Ok(core::convert::identity::<u64>(2).checked_pow(u32::try_from(core::convert::identity::<u64>(core::convert::identity::<u64>(i).saturating_sub(1))).map_err(|_| crate::ComputeError::PowExponentTooLarge)?).ok_or(crate::ComputeError::PowOverflow)?)\n}\n\n"
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
        "pub fn half(n: u64, k: u64) -> u64 {\n    core::convert::identity::<u64>(n).checked_shr(u32::try_from(core::convert::identity::<u64>(k)).unwrap_or(u32::MAX)).unwrap_or(0)\n}\n\n"
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
    assert!(out.contains("match (core::convert::identity::<u64>(x), core::convert::identity::<u64>(y)) { (__left, 0) => 0, (__left, __right) => __left / __right }"));
    assert!(out.contains("match (core::convert::identity::<u64>(x), core::convert::identity::<u64>(y)) { (__left, 0) => __left, (__left, __right) => __left % __right }"));
    assert!(out.contains("checked_shl(u32::try_from(core::convert::identity::<u64>(y)).map_err(|_| crate::ComputeError::ShiftExponentTooLarge)?).ok_or(crate::ComputeError::ShiftOverflow)?"));
    assert!(out.contains("checked_pow(u32::try_from(core::convert::identity::<u64>(y)).map_err(|_| crate::ComputeError::PowExponentTooLarge)?).ok_or(crate::ComputeError::PowOverflow)?"));
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
fn test_shared_owned_lcnf_values_are_cloned_for_record_fields_and_compile() {
    let ir = r#"
(module M
  (type "M.Bundle"
    (ctor "M.Bundle.mk"
      (left String)
      (right String)
      (first (List Nat))
      (second (List Nat))
      (names (List String))))
  (def shared () (named "M.Bundle")
    (let text (string "shared")
      (let empty (ctor "List.nil")
        (let values (ctor "List.cons" 7 empty)
          (ctor "M.Bundle.mk" text text values values empty)))))
)
"#;
    let out = generate(ir);
    assert_eq!(
        out.matches("alloc::string::String::from(\"shared\")")
            .count(),
        2
    );
    assert_eq!(out.matches("values.clone()").count(), 2);
    assert!(!out.contains("let empty ="));

    // Parsing generated text is insufficient for ownership defects: the old
    // output was valid Rust syntax but moved the same String/Vec twice.
    let directory = std::env::temp_dir().join(std::format!(
        "prod-codegen-shared-owned-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).unwrap();
    let source = directory.join("lib.rs");
    let library = directory.join("libshared_owned.rlib");
    std::fs::write(
        &source,
        std::format!(
            r#"#![no_std]
extern crate alloc;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComputeError {{ OutputTooSmall }}
{out}
"#
        ),
    )
    .unwrap();
    let compiled = std::process::Command::new("rustc")
        .args(["--edition", "2021", "--crate-type", "lib"])
        .arg(&source)
        .arg("-o")
        .arg(&library)
        .status()
        .unwrap();
    assert!(compiled.success());
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn test_string_predicates_borrow_inputs_and_literals_without_allocating() {
    let ir = r#"
(module M
  (def equalsToken ((value String)) Bool
    (eq value (string "token")))
  (def delegates ((value String)) Bool
    (call equalsToken value))
)
"#;
    let out = generate(ir);
    assert!(out.contains("pub fn equalsToken(value: &str) -> bool"));
    assert!(out.contains("value == \"token\""));
    assert!(out.contains("pub fn delegates(value: &str) -> bool"));
    assert!(out.contains("equalsToken((value).as_ref())"));
    assert!(!out.contains("alloc::string::String::from(\"token\")"));
}

#[test]
fn test_byte_literals_preserve_arbitrary_bytes_and_borrow_in_predicates() {
    let ir = r#"
(module M
  (type "M.Record" (ctor "M.Record.mk" (data Bytes)))
  (def arbitrary () Bytes (bytes 0 128 255))
  (def empty () Bytes (bytes))
  (def equalsBytes ((value Bytes)) Bool (let token (bytes 0 128 255) (eq value token)))
  (def getter ((value (named "M.Record"))) Bytes (let result (proj "M.Record" "data" value) result))
  (def equalsField ((value (named "M.Record"))) Bool (eq (proj "M.Record" "data" value) (bytes 0 128 255)))
  (def equalsGetter ((value (named "M.Record"))) Bool (eq (bytes 0 128 255) (call getter value)))
  (def equalsAlias ((value (named "M.Record")) (flag Bool)) Bool
    (let data (if flag (proj "M.Record" "data" value) (call getter value))
      (eq data (bytes 0 128 255))))
  (def literalCall () Bool (call equalsBytes (bytes 0 128 255)))
  (def literalEqual () Bool (eq (bytes) (bytes))))
"#;
    let out = generate(ir);
    assert!(out.contains("alloc::vec![0, 128, 255]"));
    assert!(out.contains("alloc::vec![]"));
    assert!(out.contains("pub fn equalsBytes(value: &[u8]) -> bool"));
    assert!(out.contains("core::convert::AsRef::<[u8]>::as_ref(&(value)) == &[0, 128, 255]"));
    assert!(out.contains("equalsBytes(&[0, 128, 255])"));
    assert_eq!(out.matches("alloc::vec!").count(), 2);
    let directory = loop {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let candidate = std::env::temp_dir().join(std::format!(
            "prod-codegen-byte-literals-{}-{nonce}",
            std::process::id()
        ));
        match std::fs::create_dir(&candidate) {
            Ok(()) => break candidate,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                panic!("cannot create isolated byte-literal regression directory: {error}")
            }
        }
    };
    let source = directory.join("main.rs");
    let executable = directory.join("byte-literals");
    std::fs::write(
        &source,
        std::format!(
            r#"extern crate alloc;
{out}
fn main() {{
    assert_eq!(arbitrary(), vec![0, 128, 255]);
    assert_eq!(empty(), Vec::<u8>::new());
    assert!(literalCall());
    assert!(literalEqual());
    for data in [vec![0, 128, 255], vec![], vec![255, 128, 0]] {{
        let expected = data == [0, 128, 255];
        let record = Record {{ data }};
        assert_eq!(equalsBytes(&record.data), expected);
        assert_eq!(equalsField(&record), expected);
        assert_eq!(equalsGetter(&record), expected);
        for flag in [false, true] {{ assert_eq!(equalsAlias(&record, flag), expected); }}
    }}
}}
"#
        ),
    )
    .unwrap();
    let compiled = std::process::Command::new("rustc")
        .args(["--edition", "2021"])
        .arg(&source)
        .arg("-o")
        .arg(&executable)
        .output()
        .unwrap();
    assert!(
        compiled.status.success(),
        "{}",
        std::string::String::from_utf8_lossy(&compiled.stderr)
    );
    assert!(std::process::Command::new(&executable)
        .status()
        .unwrap()
        .success());
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn test_borrowed_constructor_fields_and_scalar_match_binders_execute() {
    let ir = r#"
(module M
  (type "M.Source" (ctor "M.Source.mk" (bytes (List UInt8)) (name String)))
  (type "M.Target" (ctor "M.Target.mk" (bytes (List UInt8)) (duplicate (List UInt8)) (name String)))
  (type "M.Origin"
    (ctor "M.Origin.local")
    (ctor "M.Origin.inherited" (provider Nat) (evidence (List UInt8))))
  (def copyFields ((source (named "M.Source"))) (named "M.Target")
    (let bytes (proj "M.Source" "bytes" source)
      (let alias bytes
        (let name (proj "M.Source" "name" source)
          (ctor "M.Target.mk" alias alias name)))))
  (def nameAccessor ((source (named "M.Source"))) String
    (let result (proj "M.Source" "name" source) result))
  (def copyDirect ((source (named "M.Source"))) (named "M.Target")
    (ctor "M.Target.mk" (proj "M.Source" "bytes" source)
      (proj "M.Source" "bytes" source) (call nameAccessor source)))
  (def copyIf ((source (named "M.Source")) (flag Bool)) (named "M.Target")
    (let chosen (if flag (proj "M.Source" "bytes" source) (proj "M.Source" "bytes" source))
      (ctor "M.Target.mk" chosen chosen (call nameAccessor source))))
  (def copyMatch ((source (named "M.Source")) (flag Bool)) (named "M.Target")
    (let chosen (cases flag
      (alt "Bool.true" () (proj "M.Source" "bytes" source))
      (alt "Bool.false" () (proj "M.Source" "bytes" source)))
      (ctor "M.Target.mk" chosen chosen (call nameAccessor source))))
  (def copyMixedIf ((source (named "M.Source")) (flag Bool)) (named "M.Target")
    (let chosen (if flag (proj "M.Source" "bytes" source) (ctor "List.nil"))
      (ctor "M.Target.mk" chosen chosen (call nameAccessor source))))
  (def copyMixedMatch ((source (named "M.Source")) (flag Bool)) (named "M.Target")
    (let chosen (cases flag
      (alt "Bool.true" () (proj "M.Source" "bytes" source))
      (alt "Bool.false" () (ctor "List.nil")))
      (ctor "M.Target.mk" chosen chosen (call nameAccessor source))))
  (def rebuild ((origin (named "M.Origin"))) (named "M.Origin")
    (cases origin
      (alt "M.Origin.local" () (ctor "M.Origin.local"))
      (alt "M.Origin.inherited" (provider evidence)
        (ctor "M.Origin.inherited" provider evidence))))
  (def scalar ((number Nat)) Bool (eq number 7))
  (def validates ((origin (named "M.Origin"))) Bool
    (cases origin
      (alt "M.Origin.local" () false)
      (alt "M.Origin.inherited" (provider evidence) (call scalar provider))))
)
"#;
    let out = generate(ir);
    let predicate = out.split("pub fn validates").nth(1).unwrap();
    assert!(!predicate.contains("ToOwned"));
    assert!(!predicate.contains("evidence.clone()"));
    let directory = loop {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let candidate = std::env::temp_dir().join(std::format!(
            "prod-codegen-borrowed-constructors-{}-{nonce}",
            std::process::id()
        ));
        match std::fs::create_dir(&candidate) {
            Ok(()) => break candidate,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => panic!("cannot create isolated codegen regression directory: {error}"),
        }
    };
    let source = directory.join("main.rs");
    let executable = directory.join("borrowed-constructors");
    std::fs::write(
        &source,
        std::format!(
            r#"extern crate alloc;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComputeError {{ OutputTooSmall }}
{out}
fn main() {{
    let source = Source {{ bytes: vec![1, 2, 3], name: String::from("source") }};
    let target = copyFields(&source);
    assert_eq!(target.bytes, source.bytes);
    assert_eq!(target.duplicate, source.bytes);
    assert_eq!(target.name, source.name);
    assert_ne!(target.bytes.as_ptr(), source.bytes.as_ptr());
    assert_ne!(target.bytes.as_ptr(), target.duplicate.as_ptr());
    assert_eq!(copyDirect(&source), target);
    for flag in [false, true] {{
        assert_eq!(copyIf(&source, flag), target);
        assert_eq!(copyMatch(&source, flag), target);
        let expected = if flag {{ source.bytes.clone() }} else {{ vec![] }};
        assert_eq!(copyMixedIf(&source, flag).bytes, expected);
        assert_eq!(copyMixedMatch(&source, flag).bytes, expected);
    }}
    let inherited = Origin::inherited {{ provider: 7, evidence: vec![9] }};
    assert_eq!(rebuild(&inherited), inherited);
    assert!(validates(&Origin::inherited {{ provider: 7, evidence: vec![9] }}));
    assert!(!validates(&Origin::inherited {{ provider: 8, evidence: vec![9] }}));
    assert!(!validates(&Origin::local));
}}
"#
        ),
    )
    .unwrap();
    let compiled = std::process::Command::new("rustc")
        .args(["--edition", "2021"])
        .arg(&source)
        .arg("-o")
        .arg(&executable)
        .output()
        .unwrap();
    assert!(
        compiled.status.success(),
        "{}",
        std::string::String::from_utf8_lossy(&compiled.stderr)
    );
    assert!(std::process::Command::new(&executable)
        .status()
        .unwrap()
        .success());
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn test_copy_results_borrow_projected_owned_inputs() {
    let ir = r#"
(module M
  (type "M.Row"
    (ctor "M.Row.mk" (id String) (optional (Option String))))
  (def count ((value String)) Nat 1)
  (def optionalMember ((value (Option String))) Bool true)
  (def validates ((row (named "M.Row"))) Bool
    (let id (proj "M.Row" "id" row)
      (let optional (proj "M.Row" "optional" row)
        (if (eq (call count id) 1)
            (call optionalMember optional)
            false))))
)
"#;
    let out = generate(ir);
    assert!(out.contains("pub fn count(value: alloc::string::String) -> u64"));
    assert!(out.contains("fn __prod_borrowed_count(value: &str) -> u64"));
    assert!(out.contains("pub fn optionalMember(value: Option<alloc::string::String>) -> bool"));
    assert!(out.contains(
        "fn __prod_borrowed_optionalMember(value: &Option<alloc::string::String>) -> bool"
    ));
    assert!(
        out.contains("__prod_borrowed_count((id).as_ref())"),
        "{out}"
    );
    assert!(
        out.contains("__prod_borrowed_optionalMember(&(optional))"),
        "{out}"
    );

    let directory = std::env::temp_dir().join(std::format!(
        "prod-codegen-copy-result-borrows-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).unwrap();
    let source = directory.join("lib.rs");
    let library = directory.join("libcopy_result_borrows.rlib");
    std::fs::write(
        &source,
        std::format!(
            r#"#![no_std]
extern crate alloc;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComputeError {{ OutputTooSmall }}
{out}
"#
        ),
    )
    .unwrap();
    let compiled = std::process::Command::new("rustc")
        .args(["--edition", "2021", "--crate-type", "lib"])
        .arg(&source)
        .arg("-o")
        .arg(&library)
        .status()
        .unwrap();
    assert!(compiled.success());
    std::fs::remove_dir_all(directory).unwrap();
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
    assert!(out.contains("#[repr(u8)]\npub enum Flag {\n    off = 0,\n    on = 1,\n}\n"));
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
