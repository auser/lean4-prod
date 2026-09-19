//! Recoverable parser bounds must not depend on a large caller thread stack.

use prod_ir::{parser::parse_module, Expr};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn let_module(count: usize) -> String {
    let mut text = String::from("(module StackProbe (def probe () Nat ");
    for index in 0..count {
        text.push_str(&format!("(let x{index} {index} "));
    }
    text.push('0');
    text.push_str(&")".repeat(count + 2));
    text
}

fn check_chain(count: usize) {
    eprintln!("parse chain {count}");
    let source = let_module(count);
    let (rest, module) = parse_module(&source).expect("valid finite let chain");
    assert!(rest.is_empty());
    let mut expression = &module.definitions[0].body;
    for index in 0..count {
        let Expr::Let(name, value, body) = expression else {
            panic!("missing let binding {index}");
        };
        assert_eq!(name, &format!("x{index}"));
        assert_eq!(**value, Expr::Nat(index as u64));
        expression = body;
    }
    assert_eq!(*expression, Expr::Nat(0));
    eprintln!("drop chain {count}");
    drop(module);
    eprintln!("dropped chain {count}");
}

fn check_limits() {
    check_chain(4096);
    eprintln!("reject chain 4097");
    assert!(matches!(
        parse_module(&let_module(4097)),
        Err(nom::Err::Failure(error)) if error.code == nom::error::ErrorKind::TooLarge
    ));
    let deepest = let_module(4096);
    eprintln!("reject truncated 4096");
    assert!(parse_module(&deepest[..deepest.len() - 1]).is_err());
    let trailing = format!(
        "{} ;; accepted trailing comment\nremaining",
        let_module(1024)
    );
    assert_eq!(parse_module(&trailing).unwrap().0, "remaining");
    // A long spine nested in a non-spine expression shares the total-depth
    // budget; error cleanup must also drop a previously completed sibling.
    let spine = let_module(4033);
    let body = &spine["(module StackProbe (def probe () Nat ".len()..spine.len() - 2];
    let mixed = format!(
        "(module Mix (def probe () Nat {}{}{}))",
        "(add 0 ".repeat(63),
        body,
        ")".repeat(63)
    );
    assert!(parse_module(&mixed).unwrap().0.is_empty());
    let invalid = format!("(module Mix (def probe () Nat (add {} (unknown))))", body);
    assert!(parse_module(&invalid).is_err());
    for (prefix, limit) in [("(add 0 ", 64), ("(let x ", 64)] {
        eprintln!("recursive boundary {prefix}");
        let tail = if prefix == "(let x " { " 0)" } else { ")" };
        let expression = |depth| format!("{}0{}", prefix.repeat(depth), tail.repeat(depth));
        let accepted = format!("(module Bound (def probe () Nat {}))", expression(limit));
        let rejected = format!(
            "(module Bound (def probe () Nat {}))",
            expression(limit + 1)
        );
        assert!(parse_module(&accepted).is_ok(), "at-limit nested value");
        assert!(
            matches!(parse_module(&rejected), Err(nom::Err::Failure(error)) if error.code == nom::error::ErrorKind::TooLarge)
        );
    }
    let nested_type = |depth| format!("{}Nat{}", "(Option ".repeat(depth), ")".repeat(depth));
    let accepted = format!("(module Bound (def probe () {} 0))", nested_type(64));
    let rejected = format!("(module Bound (def probe () {} 0))", nested_type(65));
    assert!(parse_module(&accepted).is_ok());
    assert!(
        matches!(parse_module(&rejected), Err(nom::Err::Failure(error)) if error.code == nom::error::ErrorKind::TooLarge)
    );
    for source in [
        "(module Broken (def probe () Nat (let x 1)))",
        "(module Broken (def probe () Nat (let x 1 2 3)))",
        "(module Broken (def probe () Nat (letter x 1 2)))",
        "(module Broken (def probe () Nat (let ;; name\n)))",
    ] {
        assert!(parse_module(source).is_err());
    }
    let source = "(module Safe (def probe () String ( ;; lead\nlet ;; binder\nx (string \"(let hidden \\\"\\\")\") ;; value\nx)))";
    assert!(parse_module(source).unwrap().0.is_empty());
}

#[test]
fn exact_operator_dispatch_preserves_every_portable_ast_variant() {
    for (operator, variant, arity) in [
        ("add", "Add", 2),
        ("sub", "Sub", 2),
        ("mul", "Mul", 2),
        ("div", "Div", 2),
        ("mod", "Mod", 2),
        ("shl", "Shl", 2),
        ("shr", "Shr", 2),
        ("pow", "Pow", 2),
        ("eq", "Eq", 2),
        ("lt", "Lt", 2),
        ("le", "Le", 2),
        ("gt", "Gt", 2),
        ("checked-add", "CheckedAdd", 2),
        ("checked-sub", "CheckedSub", 2),
        ("checked-mul", "CheckedMul", 2),
        ("checked-div", "CheckedDiv", 2),
        ("bit-and", "BitAnd", 2),
        ("bit-or", "BitOr", 2),
        ("bit-xor", "BitXor", 2),
        ("checked-shl", "CheckedShl", 2),
        ("checked-shr", "CheckedShr", 2),
        ("append", "Append", 2),
        ("index", "Index", 2),
        ("compare-bytes", "CompareBytes", 2),
        ("join", "Join", 2),
        ("checked-neg", "CheckedNeg", 1),
        ("bit-not", "BitNot", 1),
        ("checked-convert", "CheckedConvert", 1),
        ("length", "Length", 1),
        ("utf8-encode", "Utf8Encode", 1),
        ("utf8-decode", "Utf8Decode", 1),
        ("parse-decimal", "ParseDecimal", 1),
        ("format-decimal", "FormatDecimal", 1),
        ("negate", "Negate", 1),
        ("if", "If", 3),
        ("slice", "Slice", 3),
        ("split-exact", "SplitExact", 3),
        ("quotient", "Quotient", 3),
        ("remainder", "Remainder", 3),
    ] {
        let arguments = (1..=arity)
            .map(|value| format!(" {value}"))
            .collect::<String>();
        let source = format!("(module Operators (def probe () Nat ({operator}{arguments})))");
        let (rest, module) = parse_module(&source).unwrap();
        assert!(rest.is_empty());
        let values = (1..=arity)
            .map(|value| serde_json::json!({"Nat":value}))
            .collect::<Vec<_>>();
        let expected = if arity == 1 {
            values[0].clone()
        } else {
            serde_json::json!(values)
        };
        assert_eq!(
            serde_json::to_value(&module.definitions[0].body).unwrap(),
            serde_json::json!({variant:expected}),
            "{operator}"
        );
        let recovered: prod_ir::Module =
            serde_json::from_slice(&serde_json::to_vec(&module).unwrap()).unwrap();
        assert_eq!(module, recovered);
        for suffix in ["extra", "-unknown"] {
            let malformed = source.replace(operator, &format!("{operator}{suffix}"));
            assert!(
                parse_module(&malformed).is_err(),
                "unknown operator accepted"
            );
        }
    }
}

#[test]
fn long_let_chain_survives_default_and_verification_stacks() {
    if let Ok(stack) = std::env::var("PROD_IR_STACK_CASE") {
        assert!(["default", "verification"].contains(&stack.as_str()));
        let builder = std::thread::Builder::new().name("parser-regression".into());
        let builder = if stack == "verification" {
            builder.stack_size(16 * 1024 * 1024)
        } else {
            builder
        };
        builder.spawn(check_limits).unwrap().join().unwrap();
        return;
    }
    for stack in ["default", "verification"] {
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "long_let_chain_survives_default_and_verification_stacks",
                "--nocapture",
            ])
            .env_remove("RUST_MIN_STACK")
            .env("PROD_IR_STACK_CASE", stack)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let started = Instant::now();
        while child.try_wait().unwrap().is_none() {
            if started.elapsed() >= Duration::from_secs(30) {
                let _ = child.kill();
                let output = child.wait_with_output().unwrap();
                panic!(
                    "{stack} parser child timed out: {}",
                    String::from_utf8_lossy(&output.stderr)
                );
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "{stack} stack child failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
