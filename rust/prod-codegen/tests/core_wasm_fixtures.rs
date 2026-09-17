//! Compile and execute both precise Bytes ABI shapes in an actual Wasm engine.
use prod_codegen::{generate_core_wasm_package, CoreWasmSpec};
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
