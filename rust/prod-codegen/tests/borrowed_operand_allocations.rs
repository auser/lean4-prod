//! Read-only uses of an owned local must not clone its allocation.

use prod_codegen::{
    generate_cargo_package, generate_core_wasm_package, CargoPackageSpec, CoreWasmSpec,
};
use prod_ir::parser::parse_module;
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use std::process::Command;

const IR: &str = r#"(module BorrowedOperands
  (type "Parcel" (ctor "Parcel.mk" (bytes Bytes) (offset Nat)))
  (def maybe_bytes ((input Bytes) (present Bool)) (Option Bytes)
    (if present (ctor "Option.some" input) (ctor "Option.none")))
  (def none_return ((input Bytes) (present Bool)) (Option Bytes)
    (let result (call maybe_bytes input present)
      (cases result
        (alt "Option.none" () result)
        (alt "Option.some" (value) (ctor "Option.some" value)))))
  (def none_return_shadow ((input Bytes) (present Bool)) (Option Bytes)
    (let result (call maybe_bytes input present)
      (cases result
        (alt "Option.some" (result) (ctor "Option.some" result))
        (alt "Option.none" () result))))
  (def none_return_shadow_reuse ((input Bytes) (present Bool)) (Option Bytes)
    (let result (call maybe_bytes input present)
      (cases result
        (alt "Option.none" () result)
        (alt "Option.some" (result)
          (ctor "Option.some" (append (call own_bytes result) result))))))
  (def none_return_shadow_join ((input Bytes) (present Bool)) (Option Bytes)
    (let result (call maybe_bytes input present)
      (cases result
        (alt "Option.none" () result)
        (alt "Option.some" (result)
          (let continuation (jp copy () result)
            (ctor "Option.some" (append (jmp copy) (jmp copy))))))))
  (def none_return_keeps_owner ((input Bytes) (present Bool)) (Option Bytes)
    (let result (call maybe_bytes input present)
      (cases result
        (alt "Option.none" () result)
        (alt "Option.some" (value)
          (cases result
            (alt "Option.none" () (ctor "Option.none"))
            (alt "Option.some" (later) (ctor "Option.some" (append value later))))))))
  (def none_return_type_constraint ((input Bytes) (present Bool)) Nat
    (let selected
      (let result (call maybe_bytes input present)
        (cases result
          (alt "Option.none" () result)
          (alt "Option.some" (value) (ctor "Option.none"))))
      (cases selected
        (alt "Option.none" () 0)
        (alt "Option.some" (value) 1))))
  (def none_return_builder ((input Bytes) (present Bool)) (List Nat)
    (let selected
      (let result (call maybe_bytes input present)
        (cases result
          (alt "Option.none" () result)
          (alt "Option.some" (value) (ctor "Option.some" value))))
      (cases selected
        (alt "Option.none" () (ctor "List.nil"))
        (alt "Option.some" (bytes) (ctor "List.cons" (length bytes) (ctor "List.nil"))))))
  (def fresh ((input Bytes)) (Option (named "Parcel"))
    (ctor "Option.some" (ctor "Parcel.mk" input 0)))
  (def parcel ((input Bytes)) (Option Bytes)
    (let result (call fresh input)
      (cases result
        (alt "Option.none" () (ctor "Option.none"))
        (alt "Option.some" (row)
          (let bytes (proj "Parcel" "bytes" row)
            (let width (length bytes)
              (let offset (proj "Parcel" "offset" row)
                (if (eq width offset)
                    (ctor "Option.some" (proj "Parcel" "bytes" row))
                    (ctor "Option.some" bytes)))))))))
  (def accepts ((input Bytes)) Bool (eq (length input) 8192))
  (def predicate ((input Bytes)) (Option Bytes)
    (if (call accepts input) (ctor "Option.some" input) (ctor "Option.none")))
  (def own_bytes ((input Bytes)) Bytes input)
  (def own_text ((input String)) String input)
  (def slice_read ((input Bytes)) (Option Bytes)
    (let local (call own_bytes input)
      (let part (slice local 0 1)
        (if (eq (length local) 0) (ctor "Option.none") part))))
  (def index_read ((input Bytes)) (Option UInt8)
    (let local (call own_bytes input)
      (let first (index local 0)
        (if (eq (length local) 0) (ctor "Option.none") first))))
  (def append_right ((input Bytes)) Bytes
    (let local (call own_bytes input)
      (if (eq (length local) 0) (bytes) (append (bytes) local))))
  (def self_append ((input Bytes)) Bytes (append input input))
  (def compare_read ((input Bytes) (other Bytes)) Ordering
    (let left (call own_bytes input)
      (let right (call own_bytes other)
        (let compared (compare-bytes left right)
          (if (eq (length left) (length right)) compared compared)))))
  (def equality_read ((input Bytes)) Bool
    (let local (call own_bytes input)
      (if (eq local input) (eq local (bytes)) false)))
  (def fresh_words ((input (List String))) (Option (List String))
    (ctor "Option.some" input))
  (def join_read ((input (List String)) (separator String)) (Option String)
    (cases (call fresh_words input)
      (alt "Option.none" () (ctor "Option.none"))
      (alt "Option.some" (words)
        (let delimiter (call own_text separator)
          (let joined (join words delimiter)
            (if (eq (length words) 0) (ctor "Option.none")
              (if (eq (length delimiter) 2)
                (ctor "Option.some" joined) (ctor "Option.none"))))))))
  (def decimal_read ((input String)) (Option UInt16)
    (let local (call own_text input)
      (let parsed (parse-decimal-as UInt16 local)
        (if (eq local (string "")) (ctor "Option.none") parsed))))
  (def consume_parcel ((value (Option (named "Parcel")))) (Option Bytes)
    (cases value
      (alt "Option.none" () (ctor "Option.none"))
      (alt "Option.some" (row) (ctor "Option.some" (proj "Parcel" "bytes" row)))))
  (def projection_then_consume ((input Bytes)) (Option Bytes)
    (let owner (ctor "Parcel.mk" input 0)
      (let viewed (proj "Parcel" "bytes" owner)
        (let offset (proj "Parcel" "offset" owner)
          (let consumed (call consume_parcel (ctor "Option.some" owner))
            (if (eq (length viewed) offset) (ctor "Option.none") consumed))))))
  (def entry ((input Bytes)) Bytes
    (cases (call none_return input true)
      (alt "Option.none" () (bytes))
      (alt "Option.some" (output)
        (cases (call parcel output)
          (alt "Option.none" () (bytes))
          (alt "Option.some" (result) result))))))"#;

const RUNNER: &str = r#"use borrowed_operands_fixture::*;
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

struct Counted;
static ALLOCATIONS: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Counted {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        unsafe { System.dealloc(pointer, layout) }
    }
    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        unsafe { System.realloc(pointer, layout, size) }
    }
}

#[global_allocator]
static ALLOCATOR: Counted = Counted;

fn measured<T>(action: impl FnOnce() -> T) -> (T, usize) {
    ALLOCATIONS.store(0, Ordering::Relaxed);
    let value = action();
    (value, ALLOCATIONS.load(Ordering::Relaxed))
}

fn main() {
    for size in [0, 1, 8192, 65536] {
        for present in [false, true] {
            let input = vec![0x5a; size];
            let (output, count) = measured(|| none_return(input, present));
            assert_eq!(count, 0, "None-arm return must not clone a Some payload, size={size}, present={present}");
            assert_eq!(output.as_deref(), present.then_some(vec![0x5a; size].as_slice()));

            let input = vec![0x5a; size];
            let (output, count) = measured(|| none_return_shadow(input, present));
            assert_eq!(count, 0, "reversed arms and shadowing, size={size}, present={present}");
            assert_eq!(output.as_deref(), present.then_some(vec![0x5a; size].as_slice()));

            let input = vec![0x5a; size];
            let (output, count) = measured(|| none_return_shadow_reuse(input, present));
            // Keep the old name-based protection for a shadowed payload that
            // is consumed by a call, then read again to append its bytes.
            assert_eq!(count, 3 * usize::from(present && size != 0), "shadowed payload reuse, size={size}, present={present}");
            assert_eq!(output.as_deref(), present.then_some(vec![0x5a; size * 2].as_slice()));

            let input = vec![0x5a; size];
            let (output, count) = measured(|| none_return_shadow_join(input, present));
            // One syntactic payload reference is consumed twice after the
            // captured join body is inlined; it must not lose clone protection.
            // Optimization may elide the temporary right-operand clone that
            // is only borrowed, but the emitted ownership must compile in both modes.
            assert!(if present && size != 0 { (3..=4).contains(&count) } else { count == 0 },
                    "captured shadowed payload reuse: allocations={count}, size={size}, present={present}");
            assert_eq!(output.as_deref(), present.then_some(vec![0x5a; size * 2].as_slice()));

            let input = vec![0x5a; size];
            let (output, count) = measured(|| none_return_keeps_owner(input, present));
            // A real owner use in the Some branch still needs the scrutinee
            // preserved; this conservative pass must not remove that clone.
            assert_eq!(count, 3 * usize::from(present && size != 0), "later owner use, size={size}, present={present}");
            assert_eq!(output.as_deref(), present.then_some(vec![0x5a; size * 2].as_slice()));

            let input = vec![0x5a; size];
            assert_eq!(none_return_type_constraint(input, present), 0);

            let input = vec![0x5a; size];
            let mut buffer = [99_u64; 1];
            let (output, count) = measured(|| none_return_builder(input, present, &mut buffer));
            assert_eq!(count, 0, "None return nested in builder, size={size}, present={present}");
            assert_eq!(output, Ok(usize::from(present)));
            assert_eq!(buffer[0], if present { size as u64 } else { 99 });
        }
        let input = vec![0x5a; size];
        ALLOCATIONS.store(0, Ordering::Relaxed);
        let output = parcel(input);
        let allocations = ALLOCATIONS.load(Ordering::Relaxed);
        assert_eq!(allocations, usize::from(size != 0),
                   "field reads must not clone the enclosing record, size={size}");
        assert_eq!(output.as_deref(), Some(vec![0x5a; size].as_slice()));

        let input = vec![0xa5; size];
        ALLOCATIONS.store(0, Ordering::Relaxed);
        let output = predicate(input);
        let allocations = ALLOCATIONS.load(Ordering::Relaxed);
        assert_eq!(allocations, usize::from(size == 8192),
                   "borrowed predicate argument must not be cloned, size={size}");
        assert_eq!(output.as_deref(), (size == 8192).then_some(vec![0xa5; size].as_slice()));

        let input = vec![0x5a; size];
        let (output, count) = measured(|| slice_read(input));
        // The returned one-byte slice owns one allocation; reading its source owns none.
        assert_eq!(count, usize::from(size != 0), "slice source, size={size}");
        assert_eq!(output.as_deref(), (size != 0).then_some([0x5a].as_slice()));

        let input = vec![0x5a; size];
        let (output, count) = measured(|| index_read(input));
        // The Copy-returning helper borrows input; own_bytes copies it once.
        assert_eq!(count, usize::from(size != 0), "index source, size={size}");
        assert_eq!(output, (size != 0).then_some(0x5a));

        let input = vec![0x5a; size];
        let (output, count) = measured(|| append_right(input));
        // Only the output buffer is allocated; the repeated right operand is borrowed.
        assert_eq!(count, usize::from(size != 0), "append right, size={size}");
        assert_eq!(output, vec![0x5a; size]);

        let input = vec![0x5a; size];
        let (output, count) = measured(|| self_append(input));
        // Preserve one owned left clone and one growth allocation, but no right clone.
        assert_eq!(count, 2 * usize::from(size != 0), "self append, size={size}");
        assert_eq!(output, vec![0x5a; size * 2]);

        let input = vec![0x5a; size];
        let (output, count) = measured(|| equality_read(&input));
        assert_eq!(count, usize::from(size != 0), "mixed and literal equality, size={size}");
        assert_eq!(output, size == 0);

        let input = vec![0x5a; size];
        let (output, count) = measured(|| projection_then_consume(input));
        // The consumed record and returned owned bytes each need a copy. The
        // original field borrow remains live after consumption of the record copy.
        assert_eq!(count, 2 * usize::from(size != 0), "project then consume, size={size}");
        assert_eq!(output.as_deref(), (size != 0).then_some(vec![0x5a; size].as_slice()));
    }

    for (left, right) in [(b"".as_slice(), b"".as_slice()), (b"a", b""),
                           (b"", b"a"), (b"a", b"b"), (b"b", b"a"), (b"ab", b"ab")] {
        let input = left.to_vec();
        let other = right.to_vec();
        let (output, count) = measured(|| compare_read(input, other));
        // Each borrowed input becomes one owned local; both comparisons borrow.
        assert_eq!(count, usize::from(!left.is_empty()) + usize::from(!right.is_empty()),
                   "compare bytes: {left:?}, {right:?}");
        assert_eq!(output, left.cmp(right));
    }

    for words in [vec![], vec!["one".to_owned()], vec!["one".to_owned(), "two".to_owned()]] {
        let expected = (!words.is_empty()).then(|| words.join("::"));
        let delimiter = "::".to_owned();
        let (output, count) = measured(|| join_read(&words, delimiter));
        // A nonempty list clone owns one vector plus its strings; the joined
        // output owns one String. The existing delimiter is moved, never cloned.
        let expected_count = if words.is_empty() { 0 } else { words.len() + 2 };
        assert_eq!(count, expected_count, "join words and delimiter: {words:?}");
        assert_eq!(output, expected);
    }

    for (text, expected, expected_count) in [
        ("", None, 0), ("0", Some(0), 2), ("256", Some(256), 2),
        ("65535", Some(65535), 2), ("0256", None, 2), ("65536", None, 1), ("x", None, 1),
    ] {
        let input = text.to_owned();
        let (output, count) = measured(|| decimal_read(input));
        // One nonempty local copy; successful parsing formats once to check canonical spelling.
        assert_eq!(count, expected_count, "decimal input: {text:?}");
        assert_eq!(output, expected);
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
            "prod-borrowed-operands-{}-{nonce}",
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
fn read_only_owned_operands_do_not_allocate_in_std_and_no_std() {
    let (remaining, module) = parse_module(IR).unwrap();
    assert!(remaining.is_empty());
    let spec = CargoPackageSpec {
        name: "borrowed-operands-fixture".into(),
        version: "0.1.0".into(),
        description: "Borrowed operand allocation regression".into(),
        repository: "https://github.com/auser/lean4-prod".into(),
        homepage: "https://github.com/auser/lean4-prod".into(),
        readme: "Borrowed operand regression fixture.\n".into(),
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
        for optimized in [false, true] {
            let library = scratch
                .0
                .join(format!("libfixture_{standard}_{optimized}.rlib"));
            let mut compiler = Command::new("rustc");
            compiler.args([
                "--edition=2021",
                "--crate-type=rlib",
                "--crate-name=borrowed_operands_fixture",
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
                    .arg(&runner)
                    .arg("--extern")
                    .arg(format!("borrowed_operands_fixture={}", library.display()))
                    .arg("-o")
                    .arg(&executable),
            );
            succeeds(&mut Command::new(&executable));
        }
    }
}

#[test]
fn read_only_owned_operands_fit_actual_wasm_memory_bound() {
    let (remaining, module) = parse_module(IR).unwrap();
    assert!(remaining.is_empty());
    let package = generate_core_wasm_package(
        &module,
        &CoreWasmSpec {
            crate_name: "borrowed-operands-guest".into(),
            entry: "entry".into(),
            export_name: "holo_run".into(),
            input_allocation_cap: 1_048_576,
            output_allocation_cap: 1_048_576,
            maximum_pages: 80,
            input_ir_sha256: format!("{:x}", Sha256::digest(IR.as_bytes())),
        },
    )
    .unwrap();
    let scratch = Scratch::new();
    for file in package.files {
        let path = scratch.0.join(file.path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, file.bytes).unwrap();
    }
    succeeds(
        Command::new("cargo")
            .current_dir(&scratch.0)
            .args(["build", "--release", "--locked", "--offline"])
            .env_remove("RUSTC_WRAPPER")
            .env("CARGO_TARGET_DIR", scratch.0.join("target")),
    );
    succeeds(
        Command::new("node")
            .arg(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/fixtures/borrowed_operands_wasm_test.mjs"
            ))
            .arg(
                scratch
                    .0
                    .join("target/wasm32-unknown-unknown/release/borrowed_operands_guest.wasm"),
            ),
    );
}
