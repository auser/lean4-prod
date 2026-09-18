//! Scalar wrapper naming is distinct from the implementation's lexical scopes.
//! C/Rust/Python call the real generated native library; Kotlin/TypeScript use
//! their existing deterministic loader contract. Wasm executes its real guest.
use super::*;
use sha2::{Digest, Sha256};

struct Case {
    name: String,
    args: Vec<i64>,
    expected: i64,
    boolean: bool,
    signed: bool,
    checked: bool,
}

fn fixture() -> (Module, Vec<Case>) {
    let parameters = [
        "type",
        "switch",
        "lib",
        "ctypes",
        "fn",
        "native",
        "raw",
        "const",
        "bool",
        "when",
        "lambda",
        "Some",
        "None",
        "Ok",
        "Err",
        "Self",
        "_",
        "BigInt",
        "eval",
        "arguments",
        "UINT64_MAX",
        "__LINE__",
        "echo",
        "value",
        "field",
        "get",
        "set",
    ];
    let mut declarations = String::new();
    let mut cases = Vec::new();
    for (index, parameter) in parameters.iter().enumerate() {
        let name = if *parameter == "echo" {
            String::from("echo")
        } else {
            format!("case{index}")
        };
        declarations.push_str(&format!(
            "(def {name} (({parameter} Nat)) Nat {parameter})\n"
        ));
        cases.push(Case {
            name,
            args: vec![index as i64 + 7],
            expected: index as i64 + 7,
            boolean: false,
            signed: false,
            checked: false,
        });
    }
    declarations.push_str("(def prod_arg_0 ((type Nat)) Nat type)\n");
    cases.push(Case {
        name: "prod_arg_0".into(),
        args: vec![47],
        expected: 47,
        boolean: false,
        signed: false,
        checked: false,
    });
    declarations
        .push_str("(def receiver ((prod_hygiene_receiver Nat)) Nat prod_hygiene_receiver)\n");
    cases.push(Case {
        name: "receiver".into(),
        args: vec![51],
        expected: 51,
        boolean: false,
        signed: false,
        checked: false,
    });
    declarations.push_str("(def positional ((type Nat) (prod_arg_0 Nat) (__prod_type Nat) (self Nat) (__prod_self Nat)) Nat (add (mul type 10000) (add (mul prod_arg_0 1000) (add (mul __prod_type 100) (add (mul self 10) __prod_self)))))\n");
    cases.push(Case {
        name: "positional".into(),
        args: vec![1, 2, 3, 4, 5],
        expected: 12345,
        boolean: false,
        signed: false,
        checked: true,
    });
    cases.push(Case {
        name: "positional".into(),
        args: vec![3, 6, 2, 9, 1],
        expected: 36291,
        boolean: false,
        signed: false,
        checked: true,
    });
    declarations.push_str("(def typedefs ((uint64_t Nat) (x Nat)) Nat (add uint64_t x))\n");
    cases.push(Case {
        name: "typedefs".into(),
        args: vec![13, 29],
        expected: 42,
        boolean: false,
        signed: false,
        checked: true,
    });
    declarations.push_str("(def checked ((RuntimeError Nat) (PROD_STATUS_OK Nat) (Result Nat) (IllegalStateException Nat) (__prod_compute_error Nat) (ProdFfi_prod_hygiene_checked_Result Nat)) Nat (add RuntimeError (add PROD_STATUS_OK (add Result (add IllegalStateException (add __prod_compute_error ProdFfi_prod_hygiene_checked_Result))))))\n");
    cases.push(Case {
        name: "checked".into(),
        args: vec![1, 2, 3, 4, 5, 6],
        expected: 21,
        boolean: false,
        signed: false,
        checked: true,
    });
    declarations.push_str("(def flag ((Number Bool) (bool Bool)) Bool (if Number bool false))\n");
    for args in [vec![1, 1], vec![1, 0], vec![0, 1]] {
        cases.push(Case {
            name: "flag".into(),
            expected: i64::from(args == [1, 1]),
            args,
            boolean: true,
            signed: false,
            checked: false,
        });
    }
    declarations.push_str("(def signed ((when Int64)) Int64 when)\n");
    cases.push(Case {
        name: "signed".into(),
        args: vec![-17],
        expected: -17,
        boolean: false,
        signed: true,
        checked: false,
    });
    let input = format!("(module Hygiene {declarations})");
    let (tail, module) = parse_module(&input).expect("valid scalar naming fixture");
    assert!(tail.is_empty());
    assert_eq!(cases.len(), 37);
    (module, cases)
}

fn name(case: &Case) -> String {
    format!("prod_hygiene_{}", case.name)
}
fn args(case: &Case, suffix: &str, boolean: [&str; 2]) -> String {
    case.args
        .iter()
        .map(|value| {
            if case.boolean {
                boolean[usize::from(*value != 0)].into()
            } else {
                format!("{value}{suffix}")
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

#[test]
fn benign_sdk_bytes_remain_stable() {
    let (_, module) = parse_module("(module Witness (def echo ((x Nat)) Nat x))").unwrap();
    let sdk = generate_sdks(&module, "witness").unwrap();
    let mut hash = Sha256::new();
    for bytes in [
        &sdk.c_header,
        &sdk.c_wrapper,
        &sdk.rust,
        &sdk.python,
        &sdk.typescript,
        &sdk.kotlin,
        &sdk.wasm,
    ] {
        hash.update((bytes.len() as u64).to_be_bytes());
        hash.update(bytes.as_bytes());
    }
    assert_eq!(
        format!("{:x}", hash.finalize()),
        "9bbc11a07c7b6214ca09079158a9dace52624630fac7379b55545167a4b2bde2"
    );
}

#[test]
fn simultaneous_duplicate_parameters_are_not_silently_rebound() {
    let (_, module) = parse_module("(module Witness (def echo ((x Nat) (x Nat)) Nat x))").unwrap();
    for result in [
        prod_codegen::generate_c_bindings(&module).map(|_| ()),
        generate_sdks(&module, "witness").map(|_| ()),
    ] {
        assert!(
            matches!(result, Err(prod_codegen::CAbiError::UnsupportedDefinition { definition, reason })
            if definition == "echo" && reason == "simultaneous parameters repeat `x`")
        );
    }
}

#[test]
fn hygienic_c_wrapper_executes_real_native_library() {
    let directory = FixtureDir::new("hygiene-c");
    let (module, cases) = fixture();
    let sdk = fixture_sdk(&module);
    build_native_library(&directory, &module, &sdk);
    fs::write(directory.file("fixture.h"), &sdk.c_header).unwrap();
    let mut source =
        String::from("#include <stdint.h>\n#include \"fixture.h\"\nint main(void) {\n");
    for (index, case) in cases.iter().enumerate() {
        let call = format!(
            "{}({})",
            name(case),
            args(case, if case.signed { "LL" } else { "" }, ["0", "1"])
        );
        if case.checked {
            source.push_str(&format!("{}_result_t r{index} = {call}; if (r{index}.status != 0 || r{index}.value != {}) return {};\n", name(case), case.expected, index + 1));
        } else {
            source.push_str(&format!(
                "if ({call} != {}) return {};\n",
                case.expected,
                index + 1
            ));
        }
    }
    source.push_str("if (prod_hygiene_checked(UINT64_MAX, 1, 0, 0, 0, 0).status != PROD_STATUS_ADD_OVERFLOW) return 99;\nreturn 0; }\n");
    fs::write(directory.file("test.c"), source).unwrap();
    run_command(
        Command::new("cc").current_dir(&directory.path).args([
            "-std=c11",
            "-Wall",
            "-Werror",
            "test.c",
            "-L.",
            "-lfixture",
            "-o",
            "test",
        ]),
        "hygienic C compile",
    );
    let mut command = Command::new(directory.file("test"));
    add_dynamic_library_path(&mut command, &directory.path);
    run_command(&mut command, "hygienic C runtime");
}

#[test]
fn hygienic_rust_wrapper_executes_real_native_library() {
    let directory = FixtureDir::new("hygiene-rust");
    let (module, cases) = fixture();
    let sdk = fixture_sdk(&module);
    build_native_library(&directory, &module, &sdk);
    fs::write(directory.file("sdk.rs"), &sdk.rust).unwrap();
    let mut source = String::from("mod sdk; fn main() {\n");
    for case in &cases {
        let expected = if case.boolean {
            (case.expected != 0).to_string()
        } else {
            case.expected.to_string()
        };
        let expected = if case.checked {
            format!("Ok({expected})")
        } else {
            expected
        };
        source.push_str(&format!(
            "assert_eq!(sdk::{}({}), {expected});\n",
            name(case),
            args(case, "", ["false", "true"])
        ));
    }
    source.push_str("assert_eq!(sdk::prod_hygiene_checked(u64::MAX, 1, 0, 0, 0, 0), Err(1)); }\n");
    fs::write(directory.file("main.rs"), source).unwrap();
    run_command(
        Command::new("rustc").current_dir(&directory.path).args([
            "--edition=2021",
            "main.rs",
            "-L",
            "native=.",
            "-o",
            "test",
        ]),
        "hygienic Rust SDK compile",
    );
    let mut command = Command::new(directory.file("test"));
    add_dynamic_library_path(&mut command, &directory.path);
    run_command(&mut command, "hygienic Rust SDK runtime");
}

#[test]
fn hygienic_python_wrapper_executes_real_native_library() {
    let directory = FixtureDir::new("hygiene-python");
    let (module, cases) = fixture();
    let sdk = fixture_sdk(&module);
    let library = build_native_library(&directory, &module, &sdk);
    fs::write(directory.file("sdk.py"), &sdk.python).unwrap();
    let mut source = format!(
        "import sdk\nlib = sdk.load_library({:?})\n",
        library.to_str().unwrap()
    );
    for case in &cases {
        let expected = if case.boolean {
            if case.expected == 0 {
                "False".into()
            } else {
                "True".into()
            }
        } else {
            case.expected.to_string()
        };
        source.push_str(&format!(
            "assert sdk.{}(lib, {}) == {expected}\n",
            name(case),
            args(case, "", ["False", "True"])
        ));
    }
    source.push_str("try:\n    sdk.prod_hygiene_checked(lib, 2**64 - 1, 1, 0, 0, 0, 0)\n    raise AssertionError('missing overflow')\nexcept RuntimeError as error:\n    assert 'status 1' in str(error)\n");
    fs::write(directory.file("test.py"), source).unwrap();
    run_command(
        Command::new("python3").arg(directory.file("test.py")),
        "hygienic Python native runtime",
    );
}

fn native_expression(case: &Case, args: &[String], kotlin: bool) -> String {
    if case.name == "positional" {
        format!(
            "{} * 10000{} + {} * 1000{} + {} * 100{} + {} * 10{} + {}",
            args[0],
            if kotlin { "L" } else { "n" },
            args[1],
            if kotlin { "L" } else { "n" },
            args[2],
            if kotlin { "L" } else { "n" },
            args[3],
            if kotlin { "L" } else { "n" },
            args[4]
        )
    } else if matches!(case.name.as_str(), "typedefs" | "checked") {
        args.join(" + ")
    } else if case.boolean {
        if kotlin {
            format!(
                "if ({}.toInt() != 0 && {}.toInt() != 0) 1 else 0",
                args[0], args[1]
            )
        } else {
            format!("Number({}) && Number({}) ? 1 : 0", args[0], args[1])
        }
    } else {
        args[0].clone()
    }
}

#[test]
fn hygienic_typescript_wrapper_executes_loader_contract() {
    let directory = FixtureDir::new("hygiene-typescript");
    let (module, cases) = fixture();
    fs::write(directory.file("sdk.ts"), fixture_sdk(&module).typescript).unwrap();
    let mut source = String::from(
        "import { bind, NativeLibrary } from './sdk';\nconst native: NativeLibrary = {\n",
    );
    let mut seen = std::collections::BTreeSet::new();
    for case in &cases {
        if !seen.insert(&case.name) {
            continue;
        }
        let parameters = (0..case.args.len())
            .map(|n| format!("v{n}"))
            .collect::<Vec<_>>();
        let values = parameters
            .iter()
            .map(|p| format!("BigInt({p})"))
            .collect::<Vec<_>>();
        let result = native_expression(case, &values, false);
        let result = if case.checked {
            format!("({{ status: v0 === 18446744073709551615n ? 1 : 0, value: {result} }})")
        } else {
            result
        };
        source.push_str(&format!(
            "{}: ({}) => {result},\n",
            name(case),
            parameters.join(", ")
        ));
    }
    source.push_str("};\nconst api = bind(native);\n");
    for case in &cases {
        let expected = if case.boolean {
            (case.expected != 0).to_string()
        } else {
            format!("{}n", case.expected)
        };
        source.push_str(&format!(
            "if (api.{}({}) !== {expected}) throw new Error('wrong {} result');\n",
            name(case),
            args(case, "n", ["false", "true"]),
            name(case)
        ));
    }
    source.push_str("let failed = false; try { api.prod_hygiene_checked(18446744073709551615n, 1n, 0n, 0n, 0n, 0n); } catch (error) { failed = String(error).includes('status 1'); } if (!failed) throw new Error('missing status');\n");
    fs::write(directory.file("test.ts"), source).unwrap();
    run_command(
        Command::new("tsc").current_dir(&directory.path).args([
            "--strict", "--target", "ES2020", "--module", "commonjs", "--outDir", "dist", "sdk.ts",
            "test.ts",
        ]),
        "hygienic TypeScript compile",
    );
    run_command(
        Command::new("node").arg(directory.file("dist/test.js")),
        "hygienic TypeScript runtime",
    );
}

#[test]
fn hygienic_kotlin_wrapper_executes_loader_contract() {
    let directory = FixtureDir::new("hygiene-kotlin");
    let (module, cases) = fixture();
    fs::write(directory.file("Sdk.kt"), fixture_sdk(&module).kotlin).unwrap();
    fs::write(directory.file("Jna.kt"), "package com.sun.jna\ninterface Library\nopen class Structure { interface ByValue; open fun getFieldOrder(): List<String> = emptyList() }\nobject Native { fun <T> load(path: String, type: Class<T>): T = error(\"fixture $path $type\") }\n").unwrap();
    let mut source = String::from("private class TestNative : Lean4ProdNative {\n");
    let mut seen = std::collections::BTreeSet::new();
    for case in &cases {
        if !seen.insert(&case.name) {
            continue;
        }
        let parameters = (0..case.args.len())
            .map(|n| format!("v{n}"))
            .collect::<Vec<_>>();
        let signature = parameters
            .iter()
            .map(|p| format!("{p}: {}", if case.boolean { "Byte" } else { "Long" }))
            .collect::<Vec<_>>()
            .join(", ");
        let result = native_expression(case, &parameters, true);
        let result_type = if case.checked {
            format!(
                "ProdHygiene{}Result",
                case.name[..1].to_uppercase() + &case.name[1..]
            )
        } else if case.boolean {
            "Byte".into()
        } else {
            "Long".into()
        };
        let result = if case.checked {
            format!("{result_type}().also {{ if (v0 == Long.MAX_VALUE) it.status = 1 else it.value = {result} }}")
        } else {
            result
        };
        source.push_str(&format!(
            "override fun {}({signature}): {result_type} = {result}\n",
            name(case)
        ));
    }
    source.push_str("}\nfun main() { val native = TestNative()\n");
    for case in &cases {
        let expected = if case.boolean {
            (case.expected != 0).to_string()
        } else {
            format!("{}L", case.expected)
        };
        source.push_str(&format!(
            "check(native.safe_{}({}){} == {expected})\n",
            name(case),
            args(case, "L", ["false", "true"]),
            if case.checked { ".getOrThrow()" } else { "" }
        ));
    }
    source.push_str("check(native.safe_prod_hygiene_checked(Long.MAX_VALUE, 1L, 0L, 0L, 0L, 0L).isFailure)\n}\n");
    fs::write(directory.file("Test.kt"), source).unwrap();
    run_command(
        Command::new("kotlinc").current_dir(&directory.path).args([
            "-Xallow-result-return-type",
            "Jna.kt",
            "Sdk.kt",
            "Test.kt",
            "-include-runtime",
            "-d",
            "test.jar",
        ]),
        "hygienic Kotlin compile",
    );
    run_command(
        Command::new("java")
            .args(["-jar"])
            .arg(directory.file("test.jar")),
        "hygienic Kotlin runtime",
    );
}

#[test]
fn hygienic_wasm_bindgen_wrapper_executes_real_guest() {
    let directory = FixtureDir::new("hygiene-wasm");
    let (module, cases) = fixture();
    fs::create_dir(directory.file("src")).unwrap();
    fs::write(directory.file("src/lib.rs"), fixture_sdk(&module).wasm).unwrap();
    fs::write(directory.file("Cargo.toml"), "[package]\nname='hygiene'\nversion='0.0.0'\nedition='2021'\n[workspace]\n[lib]\ncrate-type=['cdylib']\n[dependencies]\nwasm-bindgen='=0.2.122'\n").unwrap();
    run_command(
        Command::new("cargo")
            .current_dir(&directory.path)
            .args(["generate-lockfile", "--offline"]),
        "hygienic Wasm dependency lock",
    );
    run_command(
        Command::new("wasm-pack")
            .current_dir(&directory.path)
            .env("CARGO_TARGET_DIR", directory.file("target"))
            .args([
                "build",
                "--target",
                "nodejs",
                "--release",
                "--out-dir",
                "pkg",
                "--",
                "--locked",
                "--offline",
            ]),
        "hygienic wasm-bindgen package",
    );
    assert!(directory.file("pkg/hygiene_bg.wasm").is_file());
    assert!(directory.file("pkg/hygiene.d.ts").is_file());
    let mut source = String::from(
        "const assert = require('node:assert/strict');\nconst sdk = require('./pkg/hygiene.js');\n",
    );
    for case in &cases {
        let expected = if case.boolean {
            (case.expected != 0).to_string()
        } else {
            format!("{}n", case.expected)
        };
        source.push_str(&format!(
            "assert.equal(sdk.{}({}), {expected});\n",
            name(case),
            args(case, "n", ["false", "true"])
        ));
    }
    source.push_str("assert.throws(() => sdk.prod_hygiene_checked(18446744073709551615n, 1n, 0n, 0n, 0n, 0n), /addition overflow/);\n");
    fs::write(directory.file("test.cjs"), source).unwrap();
    run_command(
        Command::new("node").arg(directory.file("test.cjs")),
        "hygienic wasm-bindgen runtime",
    );
}
