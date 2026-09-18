//! Compile and execute wide Nat literals through the public package generator.

use prod_codegen::{generate_cargo_package, CargoPackageSpec};
use prod_ir::parser::parse_module;
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use std::process::Command;

const IR: &str = r#"(module WideNat
  (def sub32 ((x Nat)) Nat (let bound 4294967295 (sub bound x)))
  (def sub64 ((x Nat)) Nat (let bound 18446744073709551615 (sub bound x)))
  (def alias64 ((x Nat)) Nat
    (let bound 18446744073709551615 (let alias bound (sub alias x))))
  (def add_wide ((x Nat)) Nat (let bound 9223372036854775808 (add bound x)))
  (def mul_wide ((x Nat)) Nat (let bound 4294967295 (mul bound x)))
  (def shl_wide ((x Nat)) Nat (let bound 2147483648 (shl bound x)))
  (def shr_wide ((x Nat)) Nat (let bound 9223372036854775808 (shr bound x)))
  (def pow_wide ((x Nat)) Nat (let bound 2147483648 (pow bound x)))
  (def shift_exponent ((x Nat)) Nat (let bound 4294967296 (shl x bound)))
  (def power_exponent ((x Nat)) Nat (let bound 4294967296 (pow x bound)))
  (def right_exponent ((x Nat)) Nat (let bound 18446744073709551615 (shr x bound)))
  (def contextual32 ((x UInt32)) UInt32
    (let bound 4294967295 (if (eq x 0) bound x))))"#;

const RUNNER: &str = r#"use wide_nat_fixture::*;
fn main() {
    for x in [0, 1, 2, 2147483647, 2147483648, 4294967295, 4294967296,
              9223372036854775807, 9223372036854775808, u64::MAX] {
        assert_eq!(sub32(x), 4294967295_u64.saturating_sub(x));
        assert_eq!(sub64(x), u64::MAX.saturating_sub(x));
        assert_eq!(alias64(x), u64::MAX.saturating_sub(x));
        assert_eq!(add_wide(x), 9223372036854775808_u64.checked_add(x)
                   .ok_or(ComputeError::AddOverflow));
        assert_eq!(mul_wide(x), 4294967295_u64.checked_mul(x)
                   .ok_or(ComputeError::MulOverflow));
        assert_eq!(shr_wide(x), u32::try_from(x).ok()
                   .and_then(|shift| 9223372036854775808_u64.checked_shr(shift))
                   .unwrap_or(0));
    }
    for x in [0, 1, 2, 31, 64] {
        assert_eq!(shl_wide(x), 2147483648_u64.checked_shl(x as u32)
                   .ok_or(ComputeError::ShiftOverflow));
        assert_eq!(pow_wide(x), 2147483648_u64.checked_pow(x as u32)
                   .ok_or(ComputeError::PowOverflow));
    }
    assert_eq!(shl_wide(u64::MAX), Err(ComputeError::ShiftExponentTooLarge));
    assert_eq!(pow_wide(u64::MAX), Err(ComputeError::PowExponentTooLarge));
    assert_eq!(shift_exponent(1), Err(ComputeError::ShiftExponentTooLarge));
    assert_eq!(power_exponent(1), Err(ComputeError::PowExponentTooLarge));
    assert_eq!(right_exponent(u64::MAX), 0);
    assert_eq!(contextual32(0), u32::MAX);
    for x in [1, i32::MAX as u32, 2147483648, u32::MAX] {
        assert_eq!(contextual32(x), x);
    }
}
"#;

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "prod-nat-literal-width-{}-{nonce}",
            std::process::id()
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        if std::thread::panicking() {
            eprintln!("retained failing generated fixture: {}", self.0.display());
        } else {
            std::fs::remove_dir_all(&self.0).unwrap();
        }
    }
}

fn succeeds(command: &mut Command) {
    let result = command.output().unwrap();
    assert!(
        result.status.success(),
        "{command:?}\n{}\n{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
}

#[test]
fn wide_nat_let_receivers_compile_and_execute_in_std_and_no_std() {
    let (remaining, module) = parse_module(IR).unwrap();
    assert!(remaining.is_empty());
    let spec = CargoPackageSpec {
        name: "wide-nat-fixture".into(),
        version: "0.1.0".into(),
        description: "Wide Nat code generation regression".into(),
        repository: "https://github.com/auser/lean4-prod".into(),
        homepage: "https://github.com/auser/lean4-prod".into(),
        readme: "Wide Nat regression fixture.\n".into(),
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
    let source = scratch.0.join("runner.rs");
    std::fs::write(&source, RUNNER).unwrap();
    for standard in [true, false] {
        for optimized in [false, true] {
            let library = scratch
                .0
                .join(format!("libwide_{standard}_{optimized}.rlib"));
            let mut compiler = Command::new("rustc");
            compiler.args([
                "--edition=2021",
                "--crate-type=rlib",
                "--crate-name=wide_nat_fixture",
            ]);
            if standard {
                compiler.args(["--cfg", "feature=\"std\""]);
            }
            if optimized {
                compiler.args(["-C", "opt-level=3", "-C", "overflow-checks=yes"]);
            }
            succeeds(
                compiler
                    .arg(scratch.0.join("src/lib.rs"))
                    .arg("-o")
                    .arg(&library),
            );
            let executable = scratch.0.join(format!("runner_{standard}_{optimized}"));
            succeeds(
                Command::new("rustc")
                    .arg("--edition=2021")
                    .arg(&source)
                    .arg("--extern")
                    .arg(format!("wide_nat_fixture={}", library.display()))
                    .arg("-o")
                    .arg(&executable),
            );
            succeeds(&mut Command::new(&executable));
        }
    }
}
