//! Execute Nat's total division/remainder semantics and eager operands.
//! Authority: pinned Lean 4.32.1 Init/Data/Nat/Div/Basic.lean,
//! Nat.div_zero and Nat.mod_zero. Instrumentation is test-only host observation.

use prod_codegen::{generate_cargo_package, CargoPackageSpec};
use prod_ir::parser::parse_module;
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use std::process::Command;

const IR: &str = r#"(module NatDivMod
  (def divide ((left Nat) (right Nat)) Nat (div left right))
  (def remainder ((left Nat) (right Nat)) Nat (mod left right))
  (def div_errors ((left Nat) (right Nat)) Nat (div (add left 1) (mul right 2)))
  (def mod_errors ((left Nat) (right Nat)) Nat (mod (add left 1) (mul right 2)))
  (def div_observe ((left Nat) (right Nat)) Nat
    (div (ctor "observe" 1 left) (ctor "observe" 2 right)))
  (def mod_observe ((left Nat) (right Nat)) Nat
    (mod (ctor "observe" 1 left) (ctor "observe" 2 right)))
  (def div_error_then_observe ((input Nat)) Nat
    (div (add input 1) (ctor "observe" 2 0)))
  (def mod_error_then_observe ((input Nat)) Nat
    (mod (add input 1) (ctor "observe" 2 0)))
  (def div_observe_then_error ((input Nat)) Nat
    (div (ctor "observe" 1 9) (mul input 2)))
  (def mod_observe_then_error ((input Nat)) Nat
    (mod (ctor "observe" 1 9) (mul input 2)))
  (def div_traps () Nat
    (div (ctor "trap" 1) (ctor "trap" 2)))
  (def mod_traps () Nat
    (mod (ctor "trap" 1) (ctor "trap" 2)))
  (def div_trap_zero () Nat (div (ctor "trap" 1) 0))
  (def mod_trap_zero () Nat (mod (ctor "trap" 1) 0))
  (def div_error_then_trap ((input Nat)) Nat
    (div (add input 1) (ctor "trap" 2)))
  (def mod_error_then_trap ((input Nat)) Nat
    (mod (add input 1) (ctor "trap" 2)))
  (def div_trap_then_error ((input Nat)) Nat
    (div (ctor "trap" 1) (mul input 2)))
  (def mod_trap_then_error ((input Nat)) Nat
    (mod (ctor "trap" 1) (mul input 2)))
  (def div_hygiene ((__left Nat) (__right Nat)) Nat
    (div (add __right 1) __left))
  (def mod_hygiene ((__left Nat) (__right Nat)) Nat
    (mod (add __right 1) __left))
  (def unused_literal_div () Nat
    (let unused (div 18446744073709551615 1) 7))
  (def unused_literal_mod () Nat
    (let unused (mod 18446744073709551615 0) 7))
  (def nested ((left Nat) (right Nat)) Nat
    (mod (div (ctor "observe" 1 left) (ctor "observe" 2 right))
      (ctor "observe" 3 5))))"#;

const OBSERVER: &str = r#"
static TRACE: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
static CALLS: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);
pub fn reset_probe() {
    TRACE.store(0, core::sync::atomic::Ordering::SeqCst);
    CALLS.store(0, core::sync::atomic::Ordering::SeqCst);
}
pub fn probe() -> (usize, u64) {
    (CALLS.load(core::sync::atomic::Ordering::SeqCst), TRACE.load(core::sync::atomic::Ordering::SeqCst))
}
fn observe(marker: u64, value: u64) -> u64 {
    CALLS.fetch_add(1, core::sync::atomic::Ordering::SeqCst);
    TRACE.store(probe().1 * 10 + marker, core::sync::atomic::Ordering::SeqCst);
    value
}
fn trap(marker: u64) -> u64 {
    observe(marker, 0);
    panic!("test-only operand trap")
}
"#;

const RUNNER: &str = r#"use nat_div_mod_fixture::*;

#[test]
fn pinned_lean_zero_and_nonzero_semantics() {
    for left in [0, 1, 7, u64::MAX - 1, u64::MAX] {
        for right in [0, 1, 2, 7, u64::MAX] {
            assert_eq!(divide(left, right), if right == 0 { 0 } else { left / right });
            assert_eq!(remainder(left, right), if right == 0 { left } else { left % right });
        }
    }
}

#[test]
fn checked_operands_evaluate_left_to_right_even_with_zero_divisor() {
    for operation in [div_errors, mod_errors] {
        assert_eq!(operation(u64::MAX, u64::MAX), Err(ComputeError::AddOverflow));
        assert_eq!(operation(u64::MAX, 0), Err(ComputeError::AddOverflow));
        assert_eq!(operation(0, u64::MAX), Err(ComputeError::MulOverflow));
    }
    assert_eq!(div_errors(8, 2), Ok(2));
    assert_eq!(mod_errors(8, 2), Ok(1));
    assert_eq!(div_errors(8, 0), Ok(0));
    assert_eq!(mod_errors(8, 0), Ok(9));
}

#[test]
fn operands_execute_once_in_source_order() {
    for right in [0, 2] {
        for (operation, division) in [(div_observe as fn(u64, u64) -> u64, true), (mod_observe, false)] {
            reset_probe();
            let result = operation(9, right);
            assert_eq!(probe(), (2, 12));
            assert_eq!(result, if division {
                if right == 0 { 0 } else { 9 / right }
            } else if right == 0 { 9 } else { 9 % right });
        }
    }
    reset_probe();
    assert_eq!(nested(29, 2), 4);
    assert_eq!(probe(), (3, 123));
}

#[test]
fn checked_failure_stops_later_operand_and_preserves_earlier_operand() {
    for operation in [div_error_then_observe, mod_error_then_observe] {
        reset_probe();
        assert_eq!(operation(u64::MAX), Err(ComputeError::AddOverflow));
        assert_eq!(probe(), (0, 0));
    }
    for operation in [div_observe_then_error, mod_observe_then_error] {
        reset_probe();
        assert_eq!(operation(u64::MAX), Err(ComputeError::MulOverflow));
        assert_eq!(probe(), (1, 1));
    }
}

#[test]
fn traps_obey_left_to_right_order_and_zero_does_not_skip_left() {
    for operation in [div_traps, mod_traps, div_trap_zero, mod_trap_zero] {
        reset_probe();
        assert!(std::panic::catch_unwind(operation).is_err());
        assert_eq!(probe(), (1, 1));
    }
}

#[test]
fn checked_error_precedes_later_trap_and_earlier_trap_precedes_error() {
    for operation in [div_error_then_trap, mod_error_then_trap] {
        reset_probe();
        assert_eq!(operation(u64::MAX), Err(ComputeError::AddOverflow));
        assert_eq!(probe(), (0, 0));
        assert!(std::panic::catch_unwind(|| operation(0)).is_err());
        assert_eq!(probe(), (1, 2));
    }
    for operation in [div_trap_then_error, mod_trap_then_error] {
        reset_probe();
        assert!(std::panic::catch_unwind(|| operation(u64::MAX)).is_err());
        assert_eq!(probe(), (1, 1));
    }
}

#[test]
fn compiler_temporaries_do_not_capture_operand_variables() {
    assert_eq!(div_hygiene(2, 8), Ok(4));
    assert_eq!(mod_hygiene(2, 8), Ok(1));
    assert_eq!(div_hygiene(0, 8), Ok(0));
    assert_eq!(mod_hygiene(0, 8), Ok(9));
    assert_eq!(unused_literal_div(), 7);
    assert_eq!(unused_literal_mod(), 7);
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
            std::env::temp_dir().join(format!("prod-nat-div-mod-{}-{nonce}", std::process::id()));
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
fn nat_div_mod_executes_in_std_and_no_std_debug_and_optimized() {
    let (remaining, module) = parse_module(IR).unwrap();
    assert!(remaining.is_empty());
    let spec = CargoPackageSpec {
        name: "nat-div-mod-fixture".into(),
        version: "0.1.0".into(),
        description: "Nat division and remainder regression".into(),
        repository: "https://github.com/auser/lean4-prod".into(),
        homepage: "https://github.com/auser/lean4-prod".into(),
        readme: "Nat division and remainder regression fixture.\n".into(),
        license_mit: "MIT\n".into(),
        license_apache: "Apache-2.0\n".into(),
        input_sha256: format!("{:x}", Sha256::digest(IR.as_bytes())),
        dependencies: vec![],
    };
    let package = generate_cargo_package(&module, &spec).unwrap();
    assert_eq!(package, generate_cargo_package(&module, &spec).unwrap());
    let scratch = Scratch::new();
    for mut file in package.files {
        if file.path == "src/lib.rs" {
            file.bytes.extend_from_slice(OBSERVER.as_bytes());
        }
        let path = scratch.0.join(file.path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, file.bytes).unwrap();
    }
    let runner = scratch.0.join("runner.rs");
    std::fs::write(&runner, RUNNER).unwrap();
    for standard in [true, false] {
        for optimized in [false, true] {
            let library = scratch
                .0
                .join(format!("libfixture_{standard}_{optimized}.rlib"));
            let mut compiler = Command::new("rustc");
            compiler.args([
                "--edition=2021",
                "--crate-type=rlib",
                "--crate-name=nat_div_mod_fixture",
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
            let mut runner_compiler = Command::new("rustc");
            runner_compiler.args(["--edition=2021", "--test"]);
            if optimized {
                runner_compiler.args(["-C", "opt-level=3", "-C", "overflow-checks=yes"]);
            }
            succeeds(
                runner_compiler
                    .arg(&runner)
                    .arg("--extern")
                    .arg(format!("nat_div_mod_fixture={}", library.display()))
                    .arg("-o")
                    .arg(&executable),
            );
            let output = succeeds(Command::new(&executable).arg("--test-threads=1"));
            assert!(
                output.contains("7 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out"),
                "{output}"
            );
        }
    }
}
