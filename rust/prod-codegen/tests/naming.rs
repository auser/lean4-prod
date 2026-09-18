//! Real execution of lexical scopes at the public raw-IR generation boundary.
use prod_codegen::{
    generate_cargo_package, generate_core_wasm_package, generate_def, generate_module,
    CargoPackageSpec, CoreWasmSpec, Error,
};
use prod_ir::parser::parse_module;
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::PathBuf,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("lean4-prod-naming-{}-{nonce}", std::process::id()));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn run(command: &mut Command) {
    let result = command.output().unwrap();
    assert!(
        result.status.success(),
        "{command:?}\n{}\n{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
}

const SCOPES: &str = r#"(module LexicalNames
  (type "TextBox" (ctor "TextBox.mk" (text String)))
  (def prelude ((Some Nat) (None Nat) (Ok Nat) (Err Nat) (Self Nat) (_ Nat)) Nat
    (if (eq Some None) (if (eq Ok Err) (if (eq Self _) Some 0) 0) 0))
  (def constructor ((Some Nat) (None Nat)) (Option Nat)
    (if (eq Some None) (ctor "Option.some" Some) (ctor "Option.none")))
  (def pattern ((input (Option Nat))) Nat
    (cases input (alt "Option.none" () 0) (alt "Option.some" (Some) Some)))
  (def prod_local_0 ((input Bytes)) Bytes input)
  (def called ((input Bytes)) Bytes input)
  (def measured ((input String)) Nat (length input))
  (def own_helper ((__prod_borrowed_own_helper String)) Nat (length __prod_borrowed_own_helper))
  (def captures ((__value Bytes) (called Bytes) (__prod_borrowed_measured Nat)) Bytes
    (if (eq (call measured (string "abc")) __prod_borrowed_measured)
      (append (call prod_local_0 __value) (call called called)) (bytes 254)))
  (def nested ((input Bytes)) Bytes
    (let text (string "outer") (append (utf8-encode text)
      (let text (string "inner") (utf8-encode text)))))
  (def siblings ((input Bytes)) Bytes
    (append (let text (string "left") (utf8-encode text))
      (let text (string "right") (utf8-encode text))))
  (def keyword ((self Bytes) (__prod_self Bytes)) Bytes (append self __prod_self))
  (def temporary ((__value Bytes) (tail Bytes)) Bytes (append tail __value))
  (def positional ((text Bytes)) Bytes
    (let text (bytes 9) (append (param 0) text)))
  (def buffer ((output Nat)) (List Nat) (ctor "List.cons" output (ctor "List.nil")))
  (def alternatives ((input Bytes)) Bytes
    (cases (utf8-decode input) (alt "Option.none" () (bytes 255))
      (alt "Option.some" (text)
        (append (cases (ctor "Option.some" text)
          (alt "Option.none" () (bytes)) (alt "Option.some" (text) (utf8-encode text)))
          (cases (ctor "Option.some" text)
            (alt "Option.none" () (bytes)) (alt "Option.some" (text) (utf8-encode text)))))))
  (def joins ((input Bytes)) Bytes
    (let g (jp g (value) (append value (bytes 1)))
      (append (jmp g input)
        (let g (jp g (value) (append value (bytes 2))) (jmp g input)))))
  (def mixed ((record (named "TextBox")) (input Bytes)) Bytes
    (append (cases record (alt "TextBox.mk" (text) (utf8-encode text)))
      (cases (utf8-decode input) (alt "Option.none" () (bytes 255))
        (alt "Option.some" (text)
          (if (eq (split-exact text (string "|") 4) (split-exact text (string "|") 4)) input (bytes 254))))))
  (def entry ((input Bytes)) Bytes
    (cases (utf8-decode input) (alt "Option.none" () (bytes 255))
      (alt "Option.some" (text)
        (if (eq (call nested input) (bytes 111 117 116 101 114 105 110 110 101 114))
          (if (eq (call siblings input) (bytes 108 101 102 116 114 105 103 104 116))
            (if (eq (call keyword input (bytes 7)) (append input (bytes 7)))
              (if (eq (call temporary input (bytes 8)) (append (bytes 8) input))
                (if (eq (call positional input) (append input (bytes 9)))
                  (if (eq (call alternatives input) (append input input))
                    (if (eq (call joins input) (append (append input (bytes 1)) (append input (bytes 2))))
                      (if (eq (call mixed (ctor "TextBox.mk" (string "field")) input)
                        (append (bytes 102 105 101 108 100) input))
                        (if (eq (call captures input input 3) (append input input))
                          (if (eq (call prelude 42 42 3 3 5 5) 42)
                            (if (eq (call pattern (call constructor 42 42)) 42) input (bytes 254)) (bytes 254))
                          (bytes 254)) (bytes 254))
                      (bytes 254)) (bytes 254)) (bytes 254)) (bytes 254)) (bytes 254)) (bytes 254)) (bytes 254))))))"#;

#[test]
fn lexical_scopes_execute_native_no_std_and_wasm() {
    let (remaining, module) = parse_module(SCOPES).unwrap();
    assert!(remaining.trim().is_empty());
    let fixture = Fixture::new();
    let digest = format!("{:x}", Sha256::digest(SCOPES.as_bytes()));
    let native = generate_cargo_package(
        &module,
        &CargoPackageSpec {
            name: "lexical-names".into(),
            version: "0.1.0".into(),
            description: "Compiler lexical scope fixture".into(),
            repository: "https://github.com/auser/lean4-prod".into(),
            homepage: "https://github.com/auser/lean4-prod".into(),
            readme: "Compiler conformance fixture, not an application.\n".into(),
            license_mit: include_str!("fixtures/LICENSE-MIT").into(),
            license_apache: "Apache-2.0\n".into(),
            input_sha256: digest.clone(),
            dependencies: vec![],
        },
    )
    .unwrap();
    let guest = generate_core_wasm_package(
        &module,
        &CoreWasmSpec {
            crate_name: "lexical-names-guest".into(),
            entry: "entry".into(),
            export_name: "holo_run".into(),
            input_allocation_cap: 128,
            output_allocation_cap: 128,
            maximum_pages: 4,
            input_ir_sha256: digest,
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
    fs::write(native.join("tests/scopes.rs"), r#"
      use lexical_names::*;
      #[test] fn exact_scopes() {
        for text in ["", "ascii", "é\0𐐷", "\u{feff}edge"] {
          let input = text.as_bytes().to_vec();
          assert_eq!(nested(input.clone()), b"outerinner");
          assert_eq!(siblings(input.clone()), b"leftright");
          assert_eq!(keyword(input.clone(), vec![7]), [input.clone(), vec![7]].concat());
          assert_eq!(temporary(input.clone(), vec![8]), [vec![8], input.clone()].concat());
          assert_eq!(positional(input.clone()), [input.clone(), vec![9]].concat());
          assert_eq!(alternatives(input.clone()), input.repeat(2));
          assert_eq!(joins(input.clone()), [input.clone(), vec![1], input.clone(), vec![2]].concat());
          assert_eq!(mixed(&TextBox {text: "field".into()}, input.clone()), [b"field".to_vec(), input.clone()].concat());
          assert_eq!(captures(input.clone(), input.clone(), 3), input.repeat(2));
          assert_eq!(own_helper(text.into()), text.len() as u64);
          assert_eq!(entry(input.clone()), input);
        }
        let mut out = [0;1]; assert_eq!(buffer(42, &mut out).unwrap(), 1); assert_eq!(out,[42]);
        assert_eq!(prelude(42,42,3,3,5,5),42); assert_eq!(prelude(42,41,3,3,5,5),0);
        assert_eq!(constructor(42,42),Some(42)); assert_eq!(constructor(42,41),None);
        assert_eq!(pattern(Some(42)),42); assert_eq!(pattern(None),0);
        assert_eq!(entry(vec![255]), [255]);
      }
    "#).unwrap();
    for feature in [None, Some("--no-default-features")] {
        let mut command = Command::new("cargo");
        command
            .current_dir(&native)
            .args(["test", "--locked", "--offline"])
            .env_remove("RUSTC_WRAPPER")
            .env("CARGO_TARGET_DIR", fixture.0.join("native-target"));
        if let Some(feature) = feature {
            command.arg(feature);
        }
        run(&mut command);
    }
    run(Command::new("cargo")
        .current_dir(fixture.0.join("guest"))
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
                .join("wasm-target/wasm32-unknown-unknown/release/lexical_names_guest.wasm"),
        ));
}

#[test]
fn simultaneous_duplicates_fail_but_lexical_and_positional_bindings_remain_valid() {
    for body in [
        "(def invalid ((same Nat) (same Nat)) Nat same)",
        "(def invalid ((input (Tuple Nat Nat))) Nat (cases input (alt \"Prod.mk\" (same same) same)))",
        "(def invalid ((input Nat)) Nat (let g (jp g (same same) same) (jmp g input input)))",
    ] {
        let (_, module) = parse_module(&format!("(module Names {body})")).unwrap();
        assert_eq!(generate_module(&module), Err(Error::DuplicateBinding("same".into())));
        assert_eq!(generate_def(&module.definitions[0]), Err(Error::DuplicateBinding("same".into())));
    }
    let (_, module) = parse_module("(module Names (def bad ((input Nat)) Nat (param 1)))").unwrap();
    assert_eq!(
        generate_def(&module.definitions[0]),
        Err(Error::ParamOutOfBounds(1))
    );
    let (_, module) =
        parse_module("(module Names (def unchanged ((__safe Nat)) Nat __safe))").unwrap();
    assert_eq!(
        generate_def(&module.definitions[0]).unwrap(),
        "pub fn unchanged(__safe: u64) -> u64 {\n    __safe\n}\n"
    );
    let (_, module) = parse_module("(module Names (def cycle ((input Nat)) Nat (let g (jp g (value) (jmp g value)) (jmp g input))))").unwrap();
    assert!(matches!(
        generate_module(&module),
        Err(Error::UnsupportedJoinPoint(_))
    ));
}

#[test]
fn bare_host_constructor_and_pattern_names_cannot_be_captured() {
    let (_, module) = parse_module(
        r#"(module HostNames
      (def host ((HostCtor Nat) (HostValue Nat)) Nat
        (cases (ctor "HostCtor" HostCtor) (alt "HostValue" () HostValue) (default 0)))
      (def fresh ((x Nat)) Nat (let x 1 (ctor "prod_local_0" x))))"#,
    )
    .unwrap();
    let generated = generate_module(&module).unwrap();
    let fixture = Fixture::new();
    for no_std in [false, true] {
        let source = fixture.0.join("host.rs");
        fs::write(&source, format!("{}\nextern crate alloc;\n{generated}\n{}",
            if no_std {"#![no_std]"} else {""}, r#"
            #[allow(non_snake_case)] fn HostCtor(value:u64)->u64 { value + 10 }
            #[allow(non_upper_case_globals)] const HostValue:u64 = 17;
            fn prod_local_0(value:u64)->u64 { value + 1 }
            #[test] fn execute() { assert_eq!(host(7,99),99); assert_eq!(host(8,99),0); assert_eq!(fresh(100),2); }
        "#)).unwrap();
        let binary = fixture.0.join(if no_std { "no-std" } else { "std" });
        run(Command::new("rustc")
            .args(["--edition=2021", "--test"])
            .arg(&source)
            .arg("-o")
            .arg(&binary));
        run(&mut Command::new(&binary));
    }
}
