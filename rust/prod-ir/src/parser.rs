//! nom-based parser for the Lean 4 → prod compact IR format
//!
//! Grammar (simplified sexp):
//! ```text
//! module   ::= "(" "module" ident type_decl* def* ")"
//! type_decl ::= "(" "type" '"' ident '"' unsupported? ctor_decl* ")"
//! unsupported ::= "(" "unsupported" '"' text '"' ")"
//! ctor_decl ::= "(" "ctor" '"' ident '"' field* ")"
//! field    ::= "(" ident type ")"
//! def      ::= "(" "def" ident "(" param* ")" type expr ")"
//! param    ::= "(" ident type ")"
//! type     ::= "Nat" | "Int" | fixed-int | "String" | "Bytes" | "Ordering" | "Bool"
//!            | "(" "Option" type ")" | "(" "Result" type type ")" | "(" "Vec" type ")"
//!            | "(" "List" type ")" | "(" "Tuple" type* ")" | "(" "named" '"' ident '"' ")"
//!            | "(" "opaque" '"' ident '"' ")"
//! expr     ::= nat | ident | "(" "param" nat ")"
//!            | "(" "add" expr expr ")" | "(" "sub" expr expr ")" | "(" "mul" expr expr ")"
//!            | "(" "div" expr expr ")" | "(" "mod" expr expr ")" | "(" "shl" expr expr ")"
//!            | "(" "shr" expr expr ")"
//!            | "(" "pow" expr expr ")" | "(" "opaque" '"' ident '"' ")"
//!            | "(" "eq" expr expr ")" | "(" "lt" expr expr ")" | "(" "le" expr expr ")"
//!            | "(" "gt" expr expr ")" | "(" "if" expr expr expr ")" | "(" "let" ident expr expr ")"
//!            | "(" "call" ident expr* ")"
//!            | "(" "cases" expr alt* default? ")"          ; LCNF cases_on
//!            | "(" "ctor" '"' ident '"' expr* ")"          ; constructor application
//!            | "(" "proj" '"' ident '"' '"' ident '"' expr ")"  ; structure projection
//!            | "(" "jp" ident "(" ident* ")" expr ")"      ; LCNF join point
//!            | "(" "jmp" ident expr* ")"                   ; LCNF jump
//!            | "(" "unreachable" ")"
//!            | "(" "extern" '"' ident '"' expr* ")"        ; unresolved callee
//!            | portable-op | "(" "string" json-string ")"
//!            | "(" "parse-decimal-as" integer-type expr ")"
//!            | "(" "bytes" byte* ")"                     ; closed u8 literals
//! alt      ::= "(" "alt" '"' ident '"' "(" ident* ")" expr ")"
//! default  ::= "(" "default" expr ")"
//! comment  ::= ";;" ... end-of-line                       ; skipped as whitespace
//! ```

use super::{Alt, CtorDecl, Definition, Expr, Module, Type, TypeDecl};
use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;
use nom::{
    branch::alt,
    bytes::complete::{tag, take_till, take_while1},
    character::complete::{char, digit1, multispace1},
    combinator::{map, map_res, opt, peek, value},
    error::{Error as NomError, ErrorKind},
    multi::many0,
    sequence::{delimited, preceded, terminated, tuple},
    IResult,
};

/// Whitespace, including Lisp-style `;;` line comments (skipped everywhere)
fn space_and_comments(input: &str) -> IResult<&str, ()> {
    value(
        (),
        many0(alt((
            value((), multispace1),
            value((), preceded(tag(";;"), take_till(|c| c == '\n'))),
        ))),
    )(input)
}

fn ws<'a, F, O>(inner: F) -> impl FnMut(&'a str) -> IResult<&'a str, O>
where
    F: FnMut(&'a str) -> IResult<&'a str, O>,
{
    delimited(space_and_comments, inner, space_and_comments)
}

fn ident(input: &str) -> IResult<&str, String> {
    map(
        take_while1(|c: char| c.is_alphanumeric() || c == '_' || c == '-' || c == '.'),
        String::from,
    )(input)
}

fn quoted_ident(input: &str) -> IResult<&str, String> {
    delimited(char('"'), ident, char('"'))(input)
}

/// One JSON-escaped UTF-8 string. The exporter uses Lean's JSON escaper, so
/// this parser shares the same closed escape syntax instead of inventing a
/// second string grammar for the IR.
fn quoted_string(input: &str) -> IResult<&str, String> {
    if !input.starts_with('"') {
        return Err(nom::Err::Error(NomError::new(input, ErrorKind::Char)));
    }
    let mut escaped = false;
    for (index, byte) in input.as_bytes().iter().copied().enumerate().skip(1) {
        if escaped {
            escaped = false;
        } else if byte == b'\\' {
            escaped = true;
        } else if byte == b'"' {
            let raw = &input[..=index];
            let value = serde_json::from_str(raw)
                .map_err(|_| nom::Err::Error(NomError::new(input, ErrorKind::Escaped)))?;
            return Ok((&input[index + 1..], value));
        }
    }
    Err(nom::Err::Error(NomError::new(input, ErrorKind::Escaped)))
}

fn parse_u64(input: &str) -> IResult<&str, u64> {
    map_res(digit1, |s: &str| s.parse::<u64>())(input)
}

fn parse_i64(input: &str) -> IResult<&str, i64> {
    // Parse the magnitude in `i128` and apply the sign before narrowing:
    // `"9223372036854775808"` is not a valid `i64` on its own, yet
    // `-9223372036854775808` is `i64::MIN`. Parsing the digits as `i64` and
    // negating afterwards would reject that literal (and, before the digits
    // widened, panicked on the `unwrap`). Out-of-range values fail the parser
    // rather than wrapping or aborting.
    map_res(
        tuple((opt(char('-')), digit1)),
        |(neg, digits): (Option<char>, &str)| {
            let magnitude = digits.parse::<i128>().map_err(|_| ())?;
            let signed = if neg.is_some() { -magnitude } else { magnitude };
            i64::try_from(signed).map_err(|_| ())
        },
    )(input)
}

// Compiler resource limits, not a restriction on Lean's mathematical semantics.
// Let continuation spines are iterative; other recursive descent stays bounded.
const MAX_EXPRESSION_DEPTH: usize = 4096;
const MAX_RECURSIVE_DEPTH: usize = 64;
const MAX_TYPE_DEPTH: usize = 64;

fn capacity_error<T>(input: &str) -> IResult<&str, T> {
    Err(nom::Err::Failure(NomError::new(input, ErrorKind::TooLarge)))
}

fn parse_type(input: &str) -> IResult<&str, Type> {
    parse_type_at(input, MAX_TYPE_DEPTH)
}

fn parse_type_at(input: &str, remaining: usize) -> IResult<&str, Type> {
    let (input, ()) = space_and_comments(input)?;
    if remaining == 0 && input.starts_with('(') {
        return capacity_error(input);
    }
    let child = |input| parse_type_at(input, remaining.saturating_sub(1));
    ws(alt((
        value(Type::Nat, tag("Nat")),
        value(Type::Int8, tag("Int8")),
        value(Type::Int16, tag("Int16")),
        value(Type::Int32, tag("Int32")),
        value(Type::Int64, tag("Int64")),
        value(Type::UInt8, tag("UInt8")),
        value(Type::UInt16, tag("UInt16")),
        value(Type::UInt32, tag("UInt32")),
        value(Type::UInt64, tag("UInt64")),
        value(Type::Int, tag("Int")),
        value(Type::String, tag("String")),
        value(Type::Bytes, tag("Bytes")),
        value(Type::Ordering, tag("Ordering")),
        value(Type::Bool, tag("Bool")),
        map(
            delimited(char('('), tuple((tag("Option"), child)), char(')')),
            |(_, t)| Type::Option(Box::new(t)),
        ),
        map(
            delimited(char('('), tuple((tag("Result"), child, child)), char(')')),
            |(_, ok, error)| Type::Result {
                ok: Box::new(ok),
                error: Box::new(error),
            },
        ),
        map(
            delimited(char('('), tuple((tag("Vec"), child)), char(')')),
            |(_, t)| Type::Vec(Box::new(t)),
        ),
        map(
            delimited(char('('), tuple((tag("List"), child)), char(')')),
            |(_, t)| Type::List(Box::new(t)),
        ),
        map(
            delimited(char('('), tuple((tag("Tuple"), many0(child))), char(')')),
            |(_, ts)| Type::Tuple(ts),
        ),
        map(
            delimited(
                char('('),
                tuple((tag("named"), ws(quoted_ident))),
                char(')'),
            ),
            |(_, n)| Type::Named(n),
        ),
        map(
            delimited(
                char('('),
                tuple((tag("opaque"), ws(quoted_ident))),
                char(')'),
            ),
            |(_, s)| Type::Opaque(s),
        ),
    )))(input)
}

fn parse_param(input: &str) -> IResult<&str, (String, Type)> {
    delimited(char('('), tuple((ws(ident), ws(parse_type))), char(')'))(input)
}

fn parse_decimal_type(input: &str) -> IResult<&str, Type> {
    map_res(ws(ident), |name| match name.as_str() {
        "Int" => Ok(Type::Int),
        "Int8" => Ok(Type::Int8),
        "Int16" => Ok(Type::Int16),
        "Int32" => Ok(Type::Int32),
        "Int64" => Ok(Type::Int64),
        "UInt8" => Ok(Type::UInt8),
        "UInt16" => Ok(Type::UInt16),
        "UInt32" => Ok(Type::UInt32),
        "UInt64" => Ok(Type::UInt64),
        _ => Err(()),
    })(input)
}

/// `(binders...)` — a parenthesized list of bare identifiers
fn parse_binders(input: &str) -> IResult<&str, Vec<String>> {
    delimited(char('('), many0(ws(ident)), char(')'))(input)
}

/// `(alt "CtorName" (binders...) <body>)`
fn parse_alt(input: &str, depth: usize, recursion: usize) -> IResult<&str, Alt> {
    map(
        delimited(
            char('('),
            tuple((
                tag("alt"),
                ws(quoted_ident),
                ws(parse_binders),
                ws(|input| parse_expr_at(input, depth, recursion)),
            )),
            char(')'),
        ),
        |(_, ctor, binders, body)| Alt {
            ctor,
            binders,
            body,
        },
    )(input)
}

/// `(default <body>)`
fn parse_default(input: &str, depth: usize, recursion: usize) -> IResult<&str, Expr> {
    map(
        delimited(
            char('('),
            tuple((
                tag("default"),
                ws(|input| parse_expr_at(input, depth, recursion)),
            )),
            char(')'),
        ),
        |(_, body)| body,
    )(input)
}

fn parse_expr(input: &str) -> IResult<&str, Expr> {
    parse_expr_at(input, MAX_EXPRESSION_DEPTH, MAX_RECURSIVE_DEPTH)
}

fn parse_let_prefix(input: &str) -> IResult<&str, &str> {
    preceded(
        char('('),
        preceded(
            space_and_comments,
            terminated(tag("let"), peek(alt((multispace1, tag(";;"))))),
        ),
    )(input)
}

fn parse_expr_at(mut input: &str, mut depth: usize, recursion: usize) -> IResult<&str, Expr> {
    let mut bindings = Vec::new();
    loop {
        (input, ()) = space_and_comments(input)?;
        let Ok((after_keyword, _)) = parse_let_prefix(input) else {
            break;
        };
        if depth == 0 || recursion == 0 {
            return capacity_error(input);
        }
        let (after_name, name) = ws(ident)(after_keyword)?;
        let (after_value, value) = parse_expr_at(after_name, depth - 1, recursion - 1)?;
        bindings.push((name, value));
        input = after_value;
        depth -= 1;
    }
    let (mut rest, mut body) = if input.starts_with('(') {
        if depth == 0 || recursion == 0 {
            return capacity_error(input);
        }
        parse_paren_expr(input, depth - 1, recursion - 1)?
    } else {
        ws(alt((
            map(parse_u64, Expr::Nat),
            map(parse_i64, Expr::Int),
            map(tag("true"), |_| Expr::Bool(true)),
            map(tag("false"), |_| Expr::Bool(false)),
            map(ident, Expr::Var),
        )))(input)?
    };
    for (name, value) in bindings.into_iter().rev() {
        (rest, _) = ws(char(')'))(rest)?;
        body = Expr::Let(name, Box::new(value), Box::new(body));
    }
    Ok((rest, body))
}

type UnaryExpression = fn(Box<Expr>) -> Expr;
type BinaryExpression = fn(Box<Expr>, Box<Expr>) -> Expr;
type TernaryExpression = fn(Box<Expr>, Box<Expr>, Box<Expr>) -> Expr;

fn unary_operator(name: &str) -> Option<UnaryExpression> {
    Some(match name {
        "checked-neg" => Expr::CheckedNeg,
        "bit-not" => Expr::BitNot,
        "checked-convert" => Expr::CheckedConvert,
        "length" => Expr::Length,
        "utf8-encode" => Expr::Utf8Encode,
        "utf8-decode" => Expr::Utf8Decode,
        "parse-decimal" => Expr::ParseDecimal,
        "format-decimal" => Expr::FormatDecimal,
        "negate" => Expr::Negate,
        _ => return None,
    })
}

fn binary_operator(name: &str) -> Option<BinaryExpression> {
    Some(match name {
        "add" => Expr::Add,
        "sub" => Expr::Sub,
        "mul" => Expr::Mul,
        "div" => Expr::Div,
        "mod" => Expr::Mod,
        "shl" => Expr::Shl,
        "shr" => Expr::Shr,
        "pow" => Expr::Pow,
        "eq" => Expr::Eq,
        "lt" => Expr::Lt,
        "le" => Expr::Le,
        "gt" => Expr::Gt,
        "checked-add" => Expr::CheckedAdd,
        "checked-sub" => Expr::CheckedSub,
        "checked-mul" => Expr::CheckedMul,
        "checked-div" => Expr::CheckedDiv,
        "bit-and" => Expr::BitAnd,
        "bit-or" => Expr::BitOr,
        "bit-xor" => Expr::BitXor,
        "checked-shl" => Expr::CheckedShl,
        "checked-shr" => Expr::CheckedShr,
        "append" => Expr::Append,
        "index" => Expr::Index,
        "compare-bytes" => Expr::CompareBytes,
        "join" => Expr::Join,
        _ => return None,
    })
}

fn ternary_operator(name: &str) -> Option<TernaryExpression> {
    Some(match name {
        "if" => Expr::If,
        "slice" => Expr::Slice,
        "split-exact" => Expr::SplitExact,
        "quotient" => Expr::Quotient,
        "remainder" => Expr::Remainder,
        _ => return None,
    })
}

/// Dispatch exact operator tokens without keeping every nom alternative on
/// each recursive frame. In particular, neither le/let nor parse-decimal's
/// typed form can prefix-match a different supported operator.
fn parse_paren_expr(input: &str, depth: usize, recursion: usize) -> IResult<&str, Expr> {
    let child = |input| parse_expr_at(input, depth, recursion);
    let (input, _) = char('(')(input)?;
    let (input, operator) = ws(take_while1(|c: char| {
        c.is_alphanumeric() || c == '_' || c == '-' || c == '.'
    }))(input)?;
    let (rest, expression) = if let Some(constructor) = unary_operator(operator) {
        let (rest, value) = child(input)?;
        (rest, constructor(Box::new(value)))
    } else if let Some(constructor) = binary_operator(operator) {
        let (rest, left) = child(input)?;
        let (rest, right) = child(rest)?;
        (rest, constructor(Box::new(left), Box::new(right)))
    } else if let Some(constructor) = ternary_operator(operator) {
        let (rest, first) = child(input)?;
        let (rest, second) = child(rest)?;
        let (rest, third) = child(rest)?;
        (
            rest,
            constructor(Box::new(first), Box::new(second), Box::new(third)),
        )
    } else {
        match operator {
            "param" => map(ws(parse_u64), |index| Expr::Param(index as usize))(input)?,
            "call" | "jmp" => {
                let (rest, name) = ws(ident)(input)?;
                let (rest, arguments) = many0(child)(rest)?;
                let expression = if operator == "call" {
                    Expr::Call(name, arguments)
                } else {
                    Expr::Jmp(name, arguments)
                };
                (rest, expression)
            }
            "ctor" | "extern" => {
                let (rest, name) = ws(quoted_ident)(input)?;
                let (rest, arguments) = many0(child)(rest)?;
                let expression = if operator == "ctor" {
                    Expr::Ctor(name, arguments)
                } else {
                    Expr::Extern(name, arguments)
                };
                (rest, expression)
            }
            "cases" => {
                let (rest, scrutinee) = child(input)?;
                let (rest, alternatives) =
                    many0(ws(|input| parse_alt(input, depth, recursion)))(rest)?;
                let (rest, default) =
                    opt(ws(|input| parse_default(input, depth, recursion)))(rest)?;
                (
                    rest,
                    Expr::Match {
                        scrut: Box::new(scrutinee),
                        alts: alternatives,
                        default: default.map(Box::new),
                    },
                )
            }
            "proj" => {
                let (rest, ty) = ws(quoted_ident)(input)?;
                let (rest, field) = ws(quoted_ident)(rest)?;
                let (rest, value) = child(rest)?;
                (rest, Expr::Proj(ty, field, Box::new(value)))
            }
            "jp" => {
                let (rest, name) = ws(ident)(input)?;
                let (rest, params) = ws(parse_binders)(rest)?;
                let (rest, body) = child(rest)?;
                (
                    rest,
                    Expr::Jp {
                        name,
                        params,
                        body: Box::new(body),
                    },
                )
            }
            "parse-decimal-as" => {
                let (rest, target) = parse_decimal_type(input)?;
                let (rest, value) = child(rest)?;
                (rest, Expr::ParseDecimalAs(target, Box::new(value)))
            }
            "unreachable" => (input, Expr::Unreachable),
            "opaque" => map(ws(quoted_ident), Expr::Opaque)(input)?,
            "string" => map(ws(quoted_string), Expr::String)(input)?,
            "bytes" => map(many0(ws(map_res(digit1, str::parse::<u8>))), Expr::Bytes)(input)?,
            _ => return Err(nom::Err::Error(NomError::new(input, ErrorKind::Tag))),
        }
    };
    let (rest, _) = ws(char(')'))(rest)?;
    Ok((rest, expression))
}

/// `(name Type)` — one field of a constructor declaration.
fn parse_field(input: &str) -> IResult<&str, (String, Type)> {
    delimited(char('('), tuple((ws(ident), ws(parse_type))), char(')'))(input)
}

/// `(ctor "Full.Name.mk" (field Type)...)`
fn parse_ctor_decl(input: &str) -> IResult<&str, CtorDecl> {
    map(
        delimited(
            char('('),
            tuple((tag("ctor"), ws(quoted_ident), many0(ws(parse_field)))),
            char(')'),
        ),
        |(_, name, fields)| CtorDecl { name, fields },
    )(input)
}

/// `(unsupported "reason")` — a type the exporter reached but cannot describe.
fn parse_unsupported(input: &str) -> IResult<&str, String> {
    delimited(
        char('('),
        map(tuple((tag("unsupported"), ws(quoted_reason))), |(_, r)| r),
        char(')'),
    )(input)
}

/// A double-quoted free-text reason (unlike `quoted_ident`, spaces allowed).
fn quoted_reason(input: &str) -> IResult<&str, String> {
    delimited(
        char('"'),
        map(take_till(|c| c == '"'), String::from),
        char('"'),
    )(input)
}

/// `(type "Full.Name" (ctor ...)...)` or `(type "Full.Name" (unsupported "why"))`
fn parse_type_decl(input: &str) -> IResult<&str, TypeDecl> {
    map(
        delimited(
            char('('),
            tuple((
                terminated(tag("type"), multispace1),
                ws(quoted_ident),
                opt(ws(parse_unsupported)),
                many0(ws(parse_ctor_decl)),
            )),
            char(')'),
        ),
        |(_, name, unsupported, ctors)| TypeDecl {
            name,
            ctors,
            unsupported,
        },
    )(input)
}

fn parse_definition(input: &str) -> IResult<&str, Definition> {
    let (rest, (_, name, params, ret, body)) = delimited(
        char('('),
        tuple((
            tag("def"),
            ws(ident),
            delimited(char('('), many0(ws(parse_param)), char(')')),
            ws(parse_type),
            ws(parse_expr),
        )),
        char(')'),
    )(input)?;

    Ok((
        rest,
        Definition {
            name,
            params,
            ret,
            body,
        },
    ))
}

/// Parse one IR module and return the unconsumed suffix.
///
/// # Allocation and limits
///
/// This allocating compiler boundary returns an owned AST. Expression depth is
/// limited to 4096 parenthesized expression nodes; a let continuation spine is
/// parsed iteratively. Other recursive expression descent and nested type forms
/// are limited to 64 levels. Exhaustion returns `Failure(TooLarge)` rather than
/// recursing further. These are compiler resource limits, not Lean semantics.
/// Callers requiring a complete document must also reject a nonempty suffix.
/// Downstream AST transformations remain responsible for their own stack bounds.
pub fn parse_module(input: &str) -> IResult<&str, Module> {
    let (rest, (_, name, types, definitions)) = ws(delimited(
        char('('),
        tuple((
            tag("module"),
            ws(ident),
            many0(ws(parse_type_decl)),
            many0(ws(parse_definition)),
        )),
        char(')'),
    ))(input)?;

    Ok((
        rest,
        Module {
            name,
            types,
            definitions,
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::ToString;
    use alloc::vec;

    #[test]
    fn test_parse_type_nat() {
        assert_eq!(parse_type("Nat"), Ok(("", Type::Nat)));
    }

    #[test]
    fn test_parse_type_list() {
        assert_eq!(
            parse_type("(List Nat)"),
            Ok(("", Type::List(Box::new(Type::Nat))))
        );
        assert_eq!(
            parse_type("(List (Tuple Nat Nat))"),
            Ok((
                "",
                Type::List(Box::new(Type::Tuple(alloc::vec![Type::Nat, Type::Nat])))
            ))
        );
    }

    #[test]
    fn test_parse_le() {
        let (rest, expr) = parse_expr("(le (param 0) 10)").unwrap();
        assert!(rest.is_empty());
        assert!(matches!(expr, Expr::Le(_, _)));
    }

    #[test]
    fn test_parse_le_does_not_eat_let() {
        // Regression: bare `tag("le")` prefix-matches `let`.
        let (rest, expr) = parse_expr("(let x 1 x)").unwrap();
        assert!(rest.is_empty());
        assert!(matches!(expr, Expr::Let(_, _, _)));
    }

    #[test]
    fn test_parse_i64_extremes_do_not_panic() {
        // Regression: parsing the magnitude as `i64` and negating afterwards
        // panicked on `i64::MIN`'s digit string.
        assert_eq!(
            parse_expr("-9223372036854775808"),
            Ok(("", Expr::Int(i64::MIN)))
        );
        assert_eq!(parse_expr("-1"), Ok(("", Expr::Int(-1))));
        // Non-negative literals still parse as `Nat` (the `u64` branch wins).
        assert_eq!(
            parse_expr("9223372036854775807"),
            Ok(("", Expr::Nat(i64::MAX as u64)))
        );
    }

    #[test]
    fn test_parse_i64_out_of_range_is_rejected_not_wrapped() {
        // One past `i64::MIN`: the magnitude fits `i128` but not `i64`.
        assert!(parse_i64("-9223372036854775809").is_err());
    }

    #[test]
    fn test_parse_expr_add() {
        let input = r#"(add (mul (param 0) (proj "UorAtlas.Instance" "O" (param 1))) (param 2))"#;
        let (rest, expr) = parse_expr(input).unwrap();
        assert!(rest.is_empty());
        match expr {
            Expr::Add(_, _) => {}
            _ => panic!("Expected Add, got {:?}", expr),
        }
    }

    #[test]
    fn test_parse_portable_operations_and_utf8_string() {
        assert_eq!(
            parse_expr("(checked-convert value)").unwrap().1,
            Expr::CheckedConvert(Box::new(Expr::Var("value".to_string())))
        );
        assert!(matches!(
            parse_expr("(slice value start count)").unwrap().1,
            Expr::Slice(..)
        ));
        assert!(matches!(
            parse_expr("(split-exact value delimiter maximum)")
                .unwrap()
                .1,
            Expr::SplitExact(..)
        ));
        assert_eq!(
            parse_expr(r#"(string "portable ✓\n\"ok\"")"#).unwrap().1,
            Expr::String("portable ✓\n\"ok\"".to_string())
        );
    }

    #[test]
    fn test_parse_closed_byte_literals() {
        assert_eq!(parse_expr("(bytes)").unwrap().1, Expr::Bytes(vec![]));
        assert_eq!(
            parse_expr("(bytes 0 127 128 255)").unwrap().1,
            Expr::Bytes(vec![0, 127, 128, 255])
        );
        for invalid in [
            "(bytes 256)",
            "(bytes -1)",
            "(bytes value)",
            "(bytes0)",
            "(bytes 1.2)",
        ] {
            assert!(parse_expr(invalid).is_err(), "accepted {invalid}");
        }
    }

    #[test]
    fn test_parse_class_index() {
        let input = r#"
(module UorAtlas.Kernel
  (def classIndex ((h2 Nat) (d Nat) (l Nat) (inst (named "UorAtlas.Instance"))) Nat
    (add (mul (call stride inst) h2)
         (add (mul (proj "UorAtlas.Instance" "O" inst) d) l)))
)
"#;
        let (rest, module) = parse_module(input).unwrap();
        assert!(rest.trim().is_empty());
        assert_eq!(module.name, "UorAtlas.Kernel");
        assert_eq!(module.definitions.len(), 1);
        assert_eq!(module.definitions[0].name, "classIndex");
    }

    #[test]
    fn test_parse_line_comment() {
        let input = ";; header comment\n(module M ;; trailing comment\n)";
        let (rest, module) = parse_module(input).unwrap();
        assert!(rest.trim().is_empty());
        assert_eq!(module.name, "M");
    }

    #[test]
    fn test_parse_cases() {
        let input = r#"(cases (param 0)
            (alt "Some" (v) v)
            (alt "Pair" (a b) (add a b))
            (default 0))"#;
        let (rest, expr) = parse_expr(input).unwrap();
        assert!(rest.is_empty());
        match expr {
            Expr::Match {
                scrut,
                alts,
                default,
            } => {
                assert!(matches!(*scrut, Expr::Param(0)));
                assert_eq!(alts.len(), 2);
                assert_eq!(alts[0].ctor, "Some");
                assert_eq!(alts[0].binders, vec!["v"]);
                assert_eq!(alts[1].binders, vec!["a", "b"]);
                assert!(matches!(default.as_deref(), Some(Expr::Nat(0))));
            }
            _ => panic!("Expected Match, got {:?}", expr),
        }
    }

    #[test]
    fn test_parse_ctor() {
        let (rest, expr) = parse_expr(r#"(ctor "Pair" 1 2)"#).unwrap();
        assert!(rest.is_empty());
        assert_eq!(
            expr,
            Expr::Ctor("Pair".into(), vec![Expr::Nat(1), Expr::Nat(2)])
        );
    }

    #[test]
    fn test_parse_proj() {
        let (rest, expr) = parse_expr(r#"(proj "Pair" "fst" (ctor "Pair" 1 2))"#).unwrap();
        assert!(rest.is_empty());
        match expr {
            Expr::Proj(ty, field, e) => {
                assert_eq!(ty, "Pair");
                assert_eq!(field, "fst");
                assert!(matches!(*e, Expr::Ctor(..)));
            }
            _ => panic!("Expected Proj, got {:?}", expr),
        }
    }

    #[test]
    fn test_parse_jp_jmp() {
        let (rest, expr) =
            parse_expr(r#"(jp loop (i acc) (if (lt i 10) (jmp loop (add i 1) acc) acc))"#).unwrap();
        assert!(rest.is_empty());
        match expr {
            Expr::Jp { name, params, body } => {
                assert_eq!(name, "loop");
                assert_eq!(params, vec!["i", "acc"]);
                assert!(matches!(*body, Expr::If(..)));
            }
            _ => panic!("Expected Jp, got {:?}", expr),
        }
    }

    #[test]
    fn test_parse_unreachable() {
        let (rest, expr) = parse_expr("(unreachable)").unwrap();
        assert!(rest.is_empty());
        assert_eq!(expr, Expr::Unreachable);
    }

    #[test]
    fn test_parse_shr() {
        let (rest, expr) = parse_expr("(shr n 1)").unwrap();
        assert!(rest.is_empty());
        assert!(matches!(expr, Expr::Shr(..)));
    }

    #[test]
    fn test_parse_gt() {
        let (rest, expr) = parse_expr("(gt (param 0) 1)").unwrap();
        assert!(rest.is_empty());
        assert!(matches!(expr, Expr::Gt(..)));
    }

    #[test]
    fn test_parse_pow() {
        let (rest, expr) = parse_expr("(pow 2 (sub (proj \"Instance\" \"o\" i) 1))").unwrap();
        assert!(rest.is_empty());
        match expr {
            Expr::Pow(a, b) => {
                assert_eq!(*a, Expr::Nat(2));
                assert!(matches!(*b, Expr::Sub(..)));
            }
            _ => panic!("Expected Pow, got {:?}", expr),
        }
    }

    #[test]
    fn test_parse_opaque_expr() {
        let (rest, expr) = parse_expr(r#"(opaque "f1-closure")"#).unwrap();
        assert!(rest.is_empty());
        assert_eq!(expr, Expr::Opaque("f1-closure".into()));
    }

    #[test]
    fn test_parse_opaque_type() {
        let (rest, ty) = parse_type(r#"(opaque "Foo.Bar")"#).unwrap();
        assert!(rest.is_empty());
        assert_eq!(ty, Type::Opaque("Foo.Bar".into()));
    }

    #[test]
    fn test_parse_type_decl_single_ctor() {
        let input = r#"
(module M
  (type "UorAtlas.Instance"
    (ctor "UorAtlas.Instance.mk" (q Nat) (T Nat) (O Nat)))
)
"#;
        let (rest, module) = parse_module(input).unwrap();
        assert!(rest.trim().is_empty());
        assert_eq!(module.types.len(), 1);
        assert_eq!(module.types[0].name, "UorAtlas.Instance");
        assert_eq!(module.types[0].ctors.len(), 1);
        assert_eq!(module.types[0].ctors[0].name, "UorAtlas.Instance.mk");
        assert_eq!(
            module.types[0].ctors[0].fields,
            vec![
                ("q".to_string(), Type::Nat),
                ("T".to_string(), Type::Nat),
                ("O".to_string(), Type::Nat),
            ]
        );
    }

    #[test]
    fn test_parse_type_decl_multi_ctor_and_named_type() {
        let input = r#"
(module M
  (type "M.Shape"
    (ctor "M.Shape.circle" (radius Nat))
    (ctor "M.Shape.rect" (w Nat) (h Nat)))
  (def area ((s (named "M.Shape"))) Nat 0)
)
"#;
        let (rest, module) = parse_module(input).unwrap();
        assert!(rest.trim().is_empty());
        assert_eq!(module.types[0].ctors.len(), 2);
        assert_eq!(module.types[0].ctors[1].fields.len(), 2);
        assert_eq!(
            module.definitions[0].params[0].1,
            Type::Named("M.Shape".to_string())
        );
    }

    #[test]
    fn test_parse_ctor_with_no_fields() {
        let input = r#"(module M (type "M.Unit" (ctor "M.Unit.mk")))"#;
        let (_, module) = parse_module(input).unwrap();
        assert!(module.types[0].ctors[0].fields.is_empty());
    }

    #[test]
    fn test_parse_extern() {
        let (rest, expr) = parse_expr(r#"(extern "Foo.bar" 1 2)"#).unwrap();
        assert!(rest.is_empty());
        match expr {
            Expr::Extern(name, args) => {
                assert_eq!(name, "Foo.bar");
                assert_eq!(args.len(), 2);
            }
            _ => panic!("Expected Extern, got {:?}", expr),
        }
    }

    #[test]
    fn typed_decimal_target_is_preserved_and_legacy_syntax_remains_valid() {
        for (name, target) in [
            ("Int", Type::Int),
            ("Int8", Type::Int8),
            ("Int16", Type::Int16),
            ("Int32", Type::Int32),
            ("Int64", Type::Int64),
            ("UInt8", Type::UInt8),
            ("UInt16", Type::UInt16),
            ("UInt32", Type::UInt32),
            ("UInt64", Type::UInt64),
        ] {
            let source = alloc::format!("(parse-decimal-as {name} input)");
            let (rest, actual) = parse_expr(&source).unwrap();
            assert!(rest.is_empty());
            assert_eq!(
                actual,
                Expr::ParseDecimalAs(target, Box::new(Expr::Var("input".into())))
            );
        }
        let (rest, old) = parse_expr("(parse-decimal input)").unwrap();
        assert!(rest.is_empty());
        assert_eq!(old, Expr::ParseDecimal(Box::new(Expr::Var("input".into()))));
        for source in [
            "(parse-decimal-as)",
            "(parse-decimal-as UInt8)",
            "(parse-decimal-as UInt8 input extra)",
            "(parse-decimal-as UInt128 input)",
            "(parse-decimal-as UInt8input)",
            "(parse-decimal-asUInt8 input)",
            "(parse-decimal-as Nat input)",
            "(parse-decimal-as Bool input)",
            "(parse-decimal-as (Option UInt8) input)",
        ] {
            assert!(
                parse_expr(source).is_err(),
                "malformed typed decimal accepted: {source}"
            );
        }
    }

    #[test]
    fn test_parse_unsupported_type_decl() {
        let input = r#"(module M (type "M.Poly" (unsupported "type parameters")))"#;
        let (_, module) = parse_module(input).unwrap();
        assert_eq!(
            module.types[0].unsupported.as_deref(),
            Some("type parameters")
        );
        assert!(module.types[0].ctors.is_empty());
    }
}
