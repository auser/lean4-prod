//! Compile and execute both precise Bytes ABI shapes in an actual Wasm engine.
use prod_codegen::{
    generate_cargo_package, generate_core_wasm_package, CargoPackageSpec, CoreWasmSpec,
};
use prod_ir::parser::parse_module;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

struct FixtureDir(PathBuf);

impl FixtureDir {
    fn new() -> Self {
        loop {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "lean4-prod-core-wasm-{}-{nonce}",
                std::process::id()
            ));
            match fs::create_dir(&path) {
                Ok(()) => return Self(path),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("cannot create Core-Wasm fixture directory: {error}"),
            }
        }
    }
}

impl Drop for FixtureDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn run(command: &mut Command) {
    let output = command.output().expect("execute pinned devcontainer tool");
    assert!(
        output.status.success(),
        "{command:?}\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn core_wasm_bytes_entries_compile_and_execute() {
    for (ir, fallible) in [
        (
            "(module Infallible (def entry ((input Bytes)) Bytes input)
              (def unrelated () Nat (add 18446744073709551615 1)))",
            false,
        ),
        (
            "(module Direct (def entry ((input Bytes)) Bytes
              (if (eq input (bytes 255))
                (if (eq (add 18446744073709551615 1) 0) input (bytes)) input)))",
            true,
        ),
        (
            "(module Transitive
              (def entry ((input Bytes)) Bytes (call middle input))
              (def middle ((input Bytes)) Bytes (call leaf input))
              (def leaf ((input Bytes)) Bytes
                (if (eq input (bytes 255))
                  (if (eq (mul 18446744073709551615 2) 0) input (bytes)) input)))",
            true,
        ),
    ] {
        let fixture = FixtureDir::new();
        let (_, module) = parse_module(ir).unwrap();
        let package = generate_core_wasm_package(
            &module,
            &CoreWasmSpec {
                crate_name: "bytes-guest".to_owned(),
                entry: "entry".to_owned(),
                export_name: "holo_run".to_owned(),
                input_allocation_cap: 128,
                output_allocation_cap: 64,
                maximum_pages: 4,
                input_ir_sha256: format!("{:x}", Sha256::digest(ir.as_bytes())),
            },
        )
        .unwrap();
        for file in package.files {
            let path = fixture.0.join(file.path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, file.bytes).unwrap();
        }
        run(Command::new("cargo")
            .current_dir(&fixture.0)
            .args(["build", "--release", "--locked", "--offline"])
            .env_remove("RUSTC_WRAPPER")
            .env("CARGO_TARGET_DIR", fixture.0.join("target")));
        run(Command::new("node")
            .arg(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/fixtures/core_wasm_bytes_test.mjs"
            ))
            .arg(
                fixture
                    .0
                    .join("target/wasm32-unknown-unknown/release/bytes_guest.wasm"),
            )
            .arg(if fallible { "fallible" } else { "infallible" }));
    }
}

/// Scenario: a copy-returning function borrows its String argument, while
/// UTF-8 encoding still produces owned bytes without consuming that borrow.
#[test]
fn borrowed_utf8_encoding_executes_native_no_std_and_wasm() {
    let ir = r#"(module BorrowedUtf8
      (type "TextBox" (ctor "TextBox.mk" (text String)))
      (def borrowedLength ((value String)) Nat (length (utf8-encode value)))
      (def aliasedLength ((value String)) Nat
        (let alias value (length (utf8-encode alias))))
      (def repeatedLength ((value String)) Nat
        (add (length (utf8-encode value)) (length (utf8-encode value))))
      (def recordLength ((value (named "TextBox"))) Nat
        (length (utf8-encode (proj "TextBox" "text" value))))
      (def ownedEncode ((value String)) Bytes (utf8-encode value))
      (def encodeAgain ((input Bytes)) Bytes
        (cases (utf8-decode input)
          (alt "Option.none" () (bytes 255))
          (alt "Option.some" (value) (call ownedEncode value))))
      (def entry ((input Bytes)) Bytes
        (cases (utf8-decode input)
          (alt "Option.none" () (bytes 255))
          (alt "Option.some" (value)
            (if (eq (call borrowedLength value) (length input))
              (if (eq (call aliasedLength value) (length input))
                (if (eq (call repeatedLength value) (add (length input) (length input)))
                  (if (eq (call recordLength (ctor "TextBox.mk" value)) (length input))
                    (call encodeAgain input) (bytes 254)) (bytes 254)) (bytes 254)) (bytes 254))))))"#;
    execute_owned_fixture(ir, include_str!("fixtures/borrowed_utf8_generated_test.rs"));
}

/// Scenario: slice-pattern heads/tails remain borrowed until an owned
/// constructor or byte result needs them; mixed ownership never changes Eq.
#[test]
fn borrowed_lists_and_mixed_equality_execute_native_no_std_and_wasm() {
    let ir = r#"(module BorrowedCollections
      (type "TextBox" (ctor "TextBox.mk" (text String)))
      (type "TextList" (ctor "TextList.mk" (values (List String))))
      (type "ByteBox" (ctor "ByteBox.mk" (bytes Bytes)))
      (def firstBytes ((values (List String))) Bytes
        (cases values (alt "List.nil" () (bytes))
          (alt "List.cons" (head tail) (utf8-encode head))))
      (def nestedBytes ((values (List String))) Bytes
        (cases values (alt "List.nil" () (bytes))
          (alt "List.cons" (head tail)
            (cases tail (alt "List.nil" () (bytes))
              (alt "List.cons" (second rest) (utf8-encode second))))))
      (def firstBox ((values (List String))) (named "TextBox")
        (cases values (alt "List.nil" () (ctor "TextBox.mk" (string "")))
          (alt "List.cons" (head tail) (ctor "TextBox.mk" head))))
      (def aliasBox ((values (List String))) (named "TextBox")
        (let alias values
          (cases alias (alt "List.nil" () (ctor "TextBox.mk" (string "")))
            (alt "List.cons" (head tail) (ctor "TextBox.mk" head)))))
      (def prepend ((value (named "TextList"))) (named "TextList")
        (ctor "TextList.mk" (ctor "List.cons" (string "prefix") (proj "TextList" "values" value))))
      (def copyHeadAndTail ((values (List String))) (named "TextList")
        (cases values (alt "List.nil" () (ctor "TextList.mk" (ctor "List.nil")))
          (alt "List.cons" (head tail) (ctor "TextList.mk" (ctor "List.cons" head tail)))))
      (def boxBytes ((value (named "TextBox"))) Bytes
        (utf8-encode (proj "TextBox" "text" value)))
      (def sameBytes ((input Bytes) (value (named "ByteBox"))) Bool
        (eq (call ownBytes input) (proj "ByteBox" "bytes" value)))
      (def sameBytesReversed ((input Bytes) (value (named "ByteBox"))) Bool
        (eq (proj "ByteBox" "bytes" value) (call ownBytes input)))
      (def ownBytes ((input Bytes)) Bytes input)
      (def appendMatches ((input Bytes)) Bool (eq (append input (bytes)) input))
      (def ownString ((value String)) String value)
      (def sameStrings ((input String) (value (named "TextBox"))) Bool
        (eq (call ownString input) (proj "TextBox" "text" value)))
      (def entry ((input Bytes)) Bytes
        (cases (utf8-decode input)
          (alt "Option.none" () (bytes 255))
          (alt "Option.some" (text)
            (let values (ctor "List.cons" text (ctor "List.nil"))
              (let box (call firstBox values)
                (let extended (call prepend (ctor "TextList.mk" values))
                  (if (call appendMatches input)
                   (if (eq (call boxBytes (call aliasBox values)) input)
                    (if (eq (call firstBytes (proj "TextList" "values" (call copyHeadAndTail values))) input)
                   (if (eq (call boxBytes box) input)
                    (if (eq (call nestedBytes (proj "TextList" "values" extended)) input)
                      (if (call sameBytes input (ctor "ByteBox.mk" input))
                        (if (call sameBytesReversed input (ctor "ByteBox.mk" input))
                          (if (call sameStrings (proj "TextBox" "text" box) box)
                            (call firstBytes values) (bytes 254)) (bytes 254)) (bytes 254)) (bytes 254)) (bytes 254)) (bytes 254)) (bytes 254)) (bytes 254)))))))))"#;
    execute_owned_fixture(
        ir,
        include_str!("fixtures/borrowed_collections_generated_test.rs"),
    );
}

fn execute_owned_fixture(ir: &str, native_test: &str) {
    let (remaining, module) = parse_module(ir).unwrap();
    assert!(remaining.trim().is_empty());
    let input_sha256 = format!("{:x}", Sha256::digest(ir.as_bytes()));
    let fixture = FixtureDir::new();
    let native = generate_cargo_package(
        &module,
        &CargoPackageSpec {
            name: "borrowed-utf8".to_owned(),
            version: "0.1.0".to_owned(),
            description: "Generic borrowed UTF-8 compiler fixture".to_owned(),
            repository: "https://github.com/auser/lean4-prod".to_owned(),
            homepage: "https://github.com/auser/lean4-prod".to_owned(),
            readme: "Compiler conformance fixture; not an application.\n".to_owned(),
            license_mit: include_str!("fixtures/LICENSE-MIT").to_owned(),
            license_apache: "Apache-2.0\n".to_owned(),
            input_sha256: input_sha256.clone(),
            dependencies: vec![],
        },
    )
    .unwrap();
    let guest = generate_core_wasm_package(
        &module,
        &CoreWasmSpec {
            crate_name: "borrowed-utf8-guest".to_owned(),
            entry: "entry".to_owned(),
            export_name: "holo_run".to_owned(),
            input_allocation_cap: 128,
            output_allocation_cap: 128,
            maximum_pages: 4,
            input_ir_sha256: input_sha256,
        },
    )
    .unwrap();
    for (name, package) in [("native", native), ("guest", guest)] {
        for file in package.files {
            let path = fixture.0.join(name).join(file.path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, file.bytes).unwrap();
        }
    }
    let native = fixture.0.join("native");
    fs::create_dir(native.join("tests")).unwrap();
    fs::write(native.join("tests/utf8.rs"), native_test).unwrap();
    for features in [None, Some("--no-default-features")] {
        let mut command = Command::new("cargo");
        command
            .current_dir(&native)
            .args(["test", "--locked", "--offline"])
            .env_remove("RUSTC_WRAPPER")
            .env("CARGO_TARGET_DIR", fixture.0.join("native-target"));
        if let Some(flag) = features {
            command.arg(flag);
        }
        run(&mut command);
    }
    let guest = fixture.0.join("guest");
    run(Command::new("cargo")
        .current_dir(&guest)
        .args(["build", "--release", "--locked", "--offline"])
        .env_remove("RUSTC_WRAPPER")
        .env("CARGO_TARGET_DIR", fixture.0.join("wasm-target")));
    run(Command::new("node")
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/borrowed_utf8_wasm_test.mjs"
        ))
        .arg(
            fixture
                .0
                .join("wasm-target/wasm32-unknown-unknown/release/borrowed_utf8_guest.wasm"),
        ));
}
