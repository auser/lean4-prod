//! Compile/syntax fixtures for every generated SDK language.
//!
//! The fixture module is intentionally small and scalar-only. It exercises a
//! fallible function, a boolean result, and boolean argument conversion while
//! keeping the test independent of Lean/Nix-generated files.

use prod_codegen::generate_sdks;
use prod_ir::parser::parse_module;
use std::ffi::OsString;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

struct FixtureDir {
    path: PathBuf,
}

impl FixtureDir {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock is before Unix epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("lean4-prod-sdk-fixture-{nonce}"));
        fs::create_dir(&path).expect("create SDK fixture directory");
        Self { path }
    }

    fn file(&self, name: &str) -> PathBuf {
        self.path.join(name)
    }
}

impl Drop for FixtureDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn tool_available(program: &str) -> bool {
    Command::new(program).arg("--version").output().is_ok()
}

fn run_tool(program: &str, args: &[OsString]) {
    if !tool_available(program) {
        eprintln!("skipping {program} SDK fixture: tool is not installed");
        return;
    }

    let output = Command::new(program)
        .args(args)
        .output()
        .unwrap_or_else(|error| panic!("run {program} SDK fixture: {error}"));
    assert!(
        output.status.success(),
        "{program} SDK fixture failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn arg(value: impl Into<OsString>) -> OsString {
    value.into()
}

#[test]
fn generated_language_fixtures_compile_or_parse() {
    let (_, module) =
        parse_module(include_str!("fixtures/sdk_scalar.ir")).expect("parse SDK fixture IR");
    let sdk = generate_sdks(&module, "fixture").expect("generate SDK fixture");
    let fixture = FixtureDir::new();

    // C: compile a caller that includes the public header. This checks the
    // declarations, result structs, scalar ABI types, and bool conversions.
    let header = fixture.file("fixture.h");
    let c_source = fixture.file("fixture.c");
    fs::write(&header, &sdk.c_header).expect("write C fixture header");
    fs::write(
        &c_source,
        r#"
#include "fixture.h"
int main(void) {
    prod_fixture_add_result_t sum = prod_fixture_add(2, 3);
    return sum.status != PROD_STATUS_OK || sum.value != 5 ||
           prod_fixture_less(2, 3) != 1 || prod_fixture_echo(0) != 0;
}
"#,
    )
    .expect("write C fixture caller");
    run_tool(
        "cc",
        &[
            arg("-std=c11"),
            arg("-Wall"),
            arg("-fsyntax-only"),
            arg("-I"),
            fixture.path.clone().into_os_string(),
            c_source.clone().into_os_string(),
        ],
    );

    // Rust: compile the safe SDK as a library. Linking is intentionally not
    // required: the native symbols are supplied by the generated C adapter
    // in the consuming crate.
    let rust_source = fixture.file("lib.rs");
    let rust_output = fixture.file("libfixture.rlib");
    fs::write(&rust_source, &sdk.rust).expect("write Rust SDK fixture");
    run_tool(
        "rustc",
        &[
            arg("--edition=2021"),
            arg("--crate-type"),
            arg("lib"),
            arg("--crate-name"),
            arg("fixture_sdk"),
            arg("-o"),
            rust_output.into_os_string(),
            rust_source.clone().into_os_string(),
        ],
    );

    // Python: compile the generated module without loading a native library.
    let python_source = fixture.file("fixture.py");
    fs::write(&python_source, &sdk.python).expect("write Python SDK fixture");
    run_tool(
        "python3",
        &[
            arg("-m"),
            arg("py_compile"),
            python_source.clone().into_os_string(),
        ],
    );

    // TypeScript: type-check the loader-neutral binding when tsc is present.
    let typescript_source = fixture.file("index.ts");
    fs::write(&typescript_source, &sdk.typescript).expect("write TypeScript SDK fixture");
    run_tool(
        "tsc",
        &[
            arg("--noEmit"),
            arg("--strict"),
            arg("--target"),
            arg("ES2020"),
            typescript_source.clone().into_os_string(),
        ],
    );

    // Kotlin: the generated source only depends on JNA's tiny public surface.
    // Compile against local stubs when kotlinc is installed so CI does not
    // need to download a JNA jar merely to check generated syntax.
    let kotlin_source = fixture.file("Lean4Prod.kt");
    let jna_stub = fixture.file("JnaStubs.kt");
    let kotlin_output = fixture.file("fixture.jar");
    fs::write(&kotlin_source, &sdk.kotlin).expect("write Kotlin SDK fixture");
    fs::write(
        &jna_stub,
        r#"
package com.sun.jna

interface Library

open class Structure {
    interface ByValue
    open fun getFieldOrder(): List<String> = emptyList()
}

object Native {
    fun <T> load(path: String, type: Class<T>): T = error("fixture stub")
}
"#,
    )
    .expect("write Kotlin JNA stubs");
    run_tool(
        "kotlinc",
        &[
            jna_stub.into_os_string(),
            kotlin_source.into_os_string(),
            arg("-d"),
            kotlin_output.into_os_string(),
        ],
    );

    // These checks remain active even when optional language toolchains are
    // absent, so a missing compiler never turns into an untested generator.
    assert!(sdk.kotlin.contains("interface Lean4ProdNative : Library"));
    assert!(sdk.kotlin.contains("object Lean4Prod"));
    assert!(sdk.typescript.contains("export function bind"));
    assert!(sdk.python.contains("import ctypes"));
    assert!(sdk.c_header.contains("#include <stdint.h>"));
    assert!(sdk.rust.contains("#[link(name = \"fixture\")]"));
    assert!(sdk.wasm.contains("use wasm_bindgen::prelude::*"));
    assert!(sdk
        .wasm
        .contains("#[wasm_bindgen(js_name = \"prod_fixture_add\")]"));
    assert!(sdk.wasm.contains("Result<u64, JsValue>"));
}
