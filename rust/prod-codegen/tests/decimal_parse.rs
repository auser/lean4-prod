//! Legacy decimal IR remains valid; typed targets fail closed before emission.
use prod_codegen::{generate_module, Error};
use prod_ir::{parser::parse_module, Definition, Expr, Module, Type};
use std::fs;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn legacy_decimal_executes_owned_borrowed_and_temporary_inputs() {
    let ir = r#"(module LegacyDecimal
      (type "TextBox" (ctor "TextBox.mk" (text String)))
      (def legacy ((value String)) (Option UInt16) (parse-decimal value))
      (def field ((value (named "TextBox"))) (Option UInt16)
        (parse-decimal (proj "TextBox" "text" value)))
      (def head ((values (List String))) (Option UInt16)
        (cases values (alt "List.nil" () (ctor "Option.none"))
          (alt "List.cons" (first rest) (parse-decimal first))))
      (def temporary () (Option UInt16) (parse-decimal (string "65535")))
      (def reuse ((input Bytes)) Bytes
        (cases (utf8-decode input)
          (alt "Option.none" () (bytes 255))
          (alt "Option.some" (text)
            (let parsed (parse-decimal text)
              (let accepted (call legacy text)
                (if (eq parsed accepted) (utf8-encode text) (bytes 254))))))))"#;
    let (_, module) = parse_module(ir).unwrap();
    let generated = generate_module(&module).unwrap();
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let directory =
        std::env::temp_dir().join(format!("lean4-prod-decimal-{}-{nonce}", std::process::id()));
    fs::create_dir(&directory).unwrap();
    struct Remove(std::path::PathBuf);
    impl Drop for Remove {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    let _remove = Remove(directory.clone());
    let assertions = r#"
      #[test] fn execute() {
        for (text, expected) in [("0", Some(0)), ("65535", Some(65535)),
          ("65536", None), ("01", None), ("+1", None), ("-1", None), ("1\0", None)] {
          assert_eq!(legacy(text.into()), expected);
          assert_eq!(field(&TextBox { text: text.into() }), expected);
          assert_eq!(head(&[text.into()]), expected);
          assert_eq!(reuse(text.as_bytes().to_vec()), text.as_bytes());
        }
        assert_eq!(head(&[]), None);
        assert_eq!(temporary(), Some(65535));
        assert_eq!(reuse(alloc::vec![255]), alloc::vec![255]);
      }
    "#;
    for no_std in [false, true] {
        let source = directory.join("lib.rs");
        fs::write(&source, format!("{}\n#![allow(non_snake_case, unused_variables)]\nextern crate alloc;\n{generated}\n{assertions}", if no_std { "#![no_std]" } else { "" })).unwrap();
        let binary = directory.join(if no_std { "no-std" } else { "std" });
        let output = Command::new("rustc")
            .args(["--edition=2021", "--test"])
            .arg(&source)
            .arg("-o")
            .arg(&binary)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let output = Command::new(&binary).output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
    }
}

#[test]
fn typed_decimal_rejects_noninteger_and_unbounded_targets() {
    for target in [
        Type::Nat,
        Type::Bool,
        Type::String,
        Type::Bytes,
        Type::Option(Box::new(Type::UInt8)),
        Type::Int,
    ] {
        let mut module = Module {
            name: "InvalidDecimal".into(),
            types: vec![],
            definitions: vec![],
        };
        module.definitions.push(Definition {
            name: "invalid".into(),
            params: vec![],
            ret: Type::Option(Box::new(Type::UInt8)),
            body: Expr::ParseDecimalAs(target.clone(), Box::new(Expr::String("1".into()))),
        });
        let error = generate_module(&module).unwrap_err();
        if target == Type::Int {
            assert_eq!(error, Error::UnboundedInt);
        } else {
            assert!(
                matches!(error, Error::OpaqueType(message) if message.starts_with("parse-decimal-as target "))
            );
        }
    }
}
