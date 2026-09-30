//! Raw-IR API regression. The unchanged source-generated Foundry closure is a
//! separate conditional integration diagnostic, not an SDK acceptance claim.
use prod_codegen::{
    generate_cargo_package, generate_core_wasm_package, generate_module, CargoPackageSpec,
    CoreWasmSpec,
};
use prod_ir::parser::parse_module;
use sha2::{Digest, Sha256};
use std::{path::PathBuf, process::Command};

const IR: &str = r#"(module ListScrutinees
  (type "Rows" (ctor "Rows.mk" (items (List Bytes))))
  (type "Pair" (ctor "Pair.mk" (rows (named "Rows")) (bytes Bytes)))
  (def parameter ((items (List Bytes))) Bytes
    (cases items (alt "List.nil" () (bytes))
      (alt "List.cons" (head tail) head)))
  (def projected ((row (named "Rows"))) Bytes
    (cases (proj "Rows" "items" row) (alt "List.nil" () (bytes))
      (alt "List.cons" (head tail) head)))
  (def projected_alias ((row (named "Rows"))) Bytes
    (let items (proj "Rows" "items" row)
      (let alias items
        (cases alias (alt "List.nil" () (bytes))
          (alt "List.cons" (head tail) head)))))
  (def nested_tail ((row (named "Rows"))) Bytes
    (let items (proj "Rows" "items" row)
      (cases items (alt "List.nil" () (bytes))
        (alt "List.cons" (head tail)
          (cases tail (alt "List.nil" () head)
            (alt "List.cons" (second rest) second))))))
  (def owned_local ((input Bytes)) Bytes
    (let items (ctor "List.cons" input (ctor "List.nil"))
      (cases items (alt "List.nil" () (bytes))
        (alt "List.cons" (head tail) head))))
  (def owned_alias ((input Bytes)) Bytes
    (let items (ctor "List.cons" input (ctor "List.nil"))
      (let alias items
        (cases alias (alt "List.nil" () (bytes))
          (alt "List.cons" (head tail) head)))))
  (def temporary ((input Bytes)) Bytes
    (cases (ctor "List.cons" input (ctor "List.nil"))
      (alt "List.nil" () (bytes)) (alt "List.cons" (head tail) head)))
  (def shadowed ((input Bytes)) Bytes
    (let input (ctor "Rows.mk" (ctor "List.cons" input (ctor "List.nil")))
      (let input (proj "Rows" "items" input)
        (cases input (alt "List.nil" () (bytes))
          (alt "List.cons" (input tail) input)))))
  (def retained_owner ((input Bytes)) (named "Pair")
    (let row (ctor "Rows.mk" (ctor "List.cons" input (ctor "List.nil")))
      (let items (proj "Rows" "items" row)
        (let first (cases items (alt "List.nil" () (bytes))
          (alt "List.cons" (head tail) head))
          (ctor "Pair.mk" row first)))))
  (def owned_tail ((row (named "Rows"))) (named "Rows")
    (let items (proj "Rows" "items" row)
      (cases items (alt "List.nil" () (ctor "Rows.mk" (ctor "List.nil")))
        (alt "List.cons" (head tail) (ctor "Rows.mk" tail)))))
  (def fresh ((input Bytes)) (named "Rows")
    (ctor "Rows.mk" (ctor "List.cons" (bytes 42) (ctor "List.cons" input (ctor "List.nil")))))
  (def temporary_owned_tail ((input Bytes)) (named "Rows")
    (cases (proj "Rows" "items" (call fresh input))
      (alt "List.nil" () (ctor "Rows.mk" (ctor "List.nil")))
      (alt "List.cons" (head tail) (ctor "Rows.mk" tail))))
  (def branch ((left (named "Rows")) (right (named "Rows")) (choose Bool)) Bytes
    (let selected (if choose (proj "Rows" "items" left) (proj "Rows" "items" right))
      (cases selected (alt "List.nil" () (bytes))
        (alt "List.cons" (head tail) head))))
  (def constants () (List Nat) (ctor "List.cons" 7 (ctor "List.nil")))
  (def static_match () Nat
    (let values (call constants)
      (cases values (alt "List.nil" () 0) (alt "List.cons" (head tail) head))))
  (def eager_error ((first Nat) (second Nat)) Nat
    (let first (add first 1)
      (let second (mul second 2)
        (let values (ctor "List.cons" first (ctor "List.cons" second (ctor "List.nil")))
          (cases values (alt "List.nil" () 0) (alt "List.cons" (head tail) 7))))))
  (def entry ((input Bytes)) Bytes
    (let row (call fresh input)
      (if (eq (call parameter (proj "Rows" "items" row)) (bytes 42))
        (if (eq (call projected_alias row) (bytes 42))
          (if (eq (call nested_tail row) input)
            (if (eq (call owned_local input) input)
              (if (eq (call owned_alias input) input)
                (if (eq (call temporary input) input)
                  (if (eq (call shadowed input) input)
                    (call projected (call temporary_owned_tail input)) (bytes 249))
                  (bytes 250)) (bytes 251)) (bytes 252)) (bytes 253)) (bytes 254)) (bytes 255))))
  (def runtime_entry ((input Bytes)) Bytes
    (if (eq input (bytes 255))
      (if (eq (call eager_error 18446744073709551615 18446744073709551615) 7) input (bytes))
      (if (eq input (bytes 254))
        (if (eq (call eager_error 0 18446744073709551615) 7) input (bytes))
        (let row (call fresh input)
          (if (eq (call branch row row true) (bytes 42))
            (if (eq (call static_match) 7)
              (if (eq (proj "Pair" "bytes" (call retained_owner input)) input)
                (if (eq (call projected (call owned_tail row)) input)
                  (call entry input) (bytes 245)) (bytes 246)) (bytes 247)) (bytes 248))))))
)"#;

const RUNNER: &str = r#"
use list_match_fixture::*;
fn main() {
    for length in [0,1,31,256,4096,65536] {
        let bytes=(0..length).map(|i|(i%251) as u8).collect::<Vec<_>>();
        let row=Rows{items:vec![vec![42],bytes.clone()]};
        assert_eq!(parameter(&row.items),vec![42]);
        assert_eq!(projected(&row),vec![42]);
        assert_eq!(projected_alias(&row),vec![42]);
        assert_eq!(nested_tail(&row),bytes);
        assert_eq!(owned_local(bytes.clone()),bytes);
        assert_eq!(owned_alias(bytes.clone()),bytes);
        assert_eq!(temporary(bytes.clone()),bytes);
        assert_eq!(shadowed(bytes.clone()),bytes);
        let retained=retained_owner(bytes.clone());
        assert_eq!(retained.rows.items,vec![bytes.clone()]);
        assert_eq!(retained.bytes,bytes);
        let tail=owned_tail(&row);
        drop(row);
        assert_eq!(tail.items,vec![bytes.clone()]);
        assert_eq!(temporary_owned_tail(bytes.clone()).items,vec![bytes.clone()]);
        assert_eq!(entry(bytes.clone()),bytes);
        assert_eq!(runtime_entry(bytes.clone()),Ok(bytes));
    }
    let empty=Rows{items:vec![]};let full=Rows{items:vec![vec![9]]};
    assert_eq!(parameter(&[]),Vec::<u8>::new());
    assert_eq!(projected_alias(&empty),Vec::<u8>::new());
    assert_eq!(owned_tail(&empty).items,Vec::<Vec<u8>>::new());
    assert_eq!(branch(&empty,&full,true),Vec::<u8>::new());
    assert_eq!(branch(&empty,&full,false),vec![9]);
    assert_eq!(static_match(),7);
    assert_eq!(eager_error(0,0),Ok(7));
    assert_eq!(eager_error(u64::MAX,u64::MAX),Err(ComputeError::AddOverflow));
    assert_eq!(eager_error(0,u64::MAX),Err(ComputeError::MulOverflow));
    assert_eq!(runtime_entry(vec![255]),Err(ComputeError::AddOverflow));
    assert_eq!(runtime_entry(vec![254]),Err(ComputeError::MulOverflow));
    println!("list scrutinees: native values, owners, aliases, branches and eager errors passed");
}
"#;

struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("prod-list-match-{}-{nonce}", std::process::id()));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        if std::thread::panicking() {
            eprintln!("retained failing fixture: {}", self.0.display());
        } else {
            std::fs::remove_dir_all(&self.0).unwrap();
        }
    }
}
fn succeeds(command: &mut Command) -> String {
    let result = command.output().unwrap();
    assert!(
        result.status.success(),
        "{command:?}\n{}\n{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    String::from_utf8(result.stdout).unwrap()
}

#[test]
fn list_scrutinees_execute_native_std_no_std_debug_optimized() {
    let (remaining, module) = parse_module(IR).unwrap();
    assert!(remaining.is_empty());
    let spec = CargoPackageSpec {
        name: "list-match-fixture".into(),
        version: "0.1.0".into(),
        description: "List match regression".into(),
        repository: "https://github.com/auser/lean4-prod".into(),
        homepage: "https://github.com/auser/lean4-prod".into(),
        readme: "Compiler fixture.\n".into(),
        license_mit: "MIT\n".into(),
        license_apache: "Apache-2.0\n".into(),
        input_sha256: format!("{:x}", Sha256::digest(IR.as_bytes())),
        dependencies: vec![],
    };
    let package = generate_cargo_package(&module, &spec).unwrap();
    assert_eq!(package, generate_cargo_package(&module, &spec).unwrap());
    let scratch = Scratch::new();
    for file in package.files {
        let path = scratch.0.join(file.path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, file.bytes).unwrap();
    }
    let runner = scratch.0.join("runner.rs");
    std::fs::write(&runner, RUNNER).unwrap();
    for standard in [true, false] {
        for optimize in [0, 3] {
            let library = scratch
                .0
                .join(format!("libfixture_{standard}_{optimize}.rlib"));
            let mut compiler = Command::new("rustc");
            compiler.args([
                "--edition=2021",
                "--crate-type=rlib",
                "--crate-name=list_match_fixture",
            ]);
            if standard {
                compiler.args(["--cfg", "feature=\"std\""]);
            }
            compiler.args([
                "-C",
                &format!("opt-level={optimize}"),
                "-C",
                "overflow-checks=yes",
                "-C",
                "debug-assertions=yes",
            ]);
            succeeds(
                compiler
                    .arg(scratch.0.join("src/lib.rs"))
                    .arg("-o")
                    .arg(&library),
            );
            let executable = scratch.0.join(format!("runner_{standard}_{optimize}"));
            succeeds(
                Command::new("rustc")
                    .arg("--edition=2021")
                    .arg(&runner)
                    .arg("--extern")
                    .arg(format!("list_match_fixture={}", library.display()))
                    .arg("-o")
                    .arg(&executable),
            );
            assert_eq!(succeeds(&mut Command::new(executable)),"list scrutinees: native values, owners, aliases, branches and eager errors passed\n");
        }
    }
}

#[test]
fn list_scrutinees_execute_import_free_debug_release_wasm() {
    let (remaining, module) = parse_module(IR).unwrap();
    assert!(remaining.is_empty());
    let spec = CoreWasmSpec {
        crate_name: "list-match-guest".into(),
        entry: "runtime_entry".into(),
        export_name: "holo_run".into(),
        input_allocation_cap: 65536,
        output_allocation_cap: 65536,
        maximum_pages: 256,
        input_ir_sha256: format!("{:x}", Sha256::digest(IR.as_bytes())),
    };
    let package = generate_core_wasm_package(&module, &spec).unwrap();
    assert_eq!(package, generate_core_wasm_package(&module, &spec).unwrap());
    let scratch = Scratch::new();
    for file in package.files {
        let path = scratch.0.join(file.path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, file.bytes).unwrap();
    }
    for release in [false, true] {
        let mut compiler = Command::new("cargo");
        compiler.arg("build");
        if release {
            compiler.arg("--release");
        }
        succeeds(
            compiler
                .current_dir(&scratch.0)
                .args(["--locked", "--offline"])
                .env_remove("RUSTC_WRAPPER")
                .env("CARGO_TARGET_DIR", scratch.0.join("target")),
        );
        assert_eq!(
            succeeds(
                Command::new("node")
                    .arg(
                        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                            .join("tests/fixtures/list_match_wasm_test.mjs")
                    )
                    .arg(scratch.0.join(format!(
                        "target/wasm32-unknown-unknown/{}/list_match_guest.wasm",
                        if release { "release" } else { "debug" }
                    )))
            ),
            "list scrutinees: 12 values and 2 eager-error Wasm executions passed\n"
        );
    }
}

#[test]
fn list_scrutinee_normalization_preserves_single_evaluation() {
    let (_, module) = parse_module(IR).unwrap();
    let generated = generate_module(&module).unwrap();
    let function = generated
        .split("pub fn temporary_owned_tail(")
        .nth(1)
        .unwrap()
        .split("\npub fn ")
        .next()
        .unwrap();
    assert_eq!(function.matches("fresh(").count(), 1, "{function}");
}
