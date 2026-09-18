//! SplitExact's declared UInt32 bound must survive let-bound Rust inference.
use prod_codegen::generate_module;
use prod_ir::parser::parse_module;
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
        let directory =
            std::env::temp_dir().join(format!("lean4-prod-split-{}-{nonce}", std::process::id()));
        fs::create_dir(&directory).unwrap();
        Self(directory)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn split_exact_preserves_uint32_bound_without_truncation() {
    let ir = r#"(module SplitMaximum
      (def maximum ((value String)) (Option (List String))
        (let bound 4294967295 (split-exact value (string "|") bound)))
      (def bounded ((value String) (delimiter String) (maximum UInt32)) (Option (List String))
        (split-exact value delimiter maximum)))"#;
    let (_, module) = parse_module(ir).unwrap();
    let generated = generate_module(&module).unwrap();
    let fixture = Fixture::new();
    let assertions = r#"
      #[test] fn execute() {
        for (value, expected) in [("", alloc::vec![""]), ("a|b", alloc::vec!["a", "b"]),
          ("|", alloc::vec!["", ""]), ("é|𐐷", alloc::vec!["é", "𐐷"])] {
          assert_eq!(maximum(value.into()).unwrap(), expected);
          assert_eq!(bounded(value.into(), "|".into(), u32::MAX).unwrap(), expected);
          assert_eq!(bounded(value.into(), "|".into(), 0), None);
          assert_eq!(bounded(value.into(), "|".into(), expected.len() as u32).unwrap(), expected);
          assert_eq!(bounded(value.into(), "|".into(), expected.len() as u32 - 1), None);
          assert_eq!(bounded(value.into(), "".into(), u32::MAX), None);
        }
      }
    "#;
    for no_std in [false, true] {
        let source = fixture.0.join("lib.rs");
        fs::write(
            &source,
            format!(
                "{}\nextern crate alloc;\n{generated}\n{assertions}",
                if no_std { "#![no_std]" } else { "" }
            ),
        )
        .unwrap();
        let binary = fixture.0.join(if no_std { "no-std" } else { "std" });
        let result = Command::new("rustc")
            .args(["--edition=2021", "--test"])
            .arg(&source)
            .arg("-o")
            .arg(&binary)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let result = Command::new(&binary).output().unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stdout)
        );
    }
    // An out-of-contract literal cannot silently wrap to zero through a cast.
    let (_, invalid) = parse_module(
        r#"(module InvalidSplit
      (def overflow ((value String)) (Option (List String))
        (let bound 4294967296 (split-exact value (string "|") bound))))"#,
    )
    .unwrap();
    let source = fixture.0.join("invalid.rs");
    fs::write(
        &source,
        format!(
            "extern crate alloc;\n{}",
            generate_module(&invalid).unwrap()
        ),
    )
    .unwrap();
    let result = Command::new("rustc")
        .args(["--edition=2021", "--crate-type=rlib"])
        .arg(&source)
        .arg("-o")
        .arg(fixture.0.join("invalid.rlib"))
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("literal out of range for `u32`"));
}
