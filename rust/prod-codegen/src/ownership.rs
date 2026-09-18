//! Recover only types explicitly available to ownership analysis. Unknown
//! expressions remain unknown; this is not target-type inference or coercion.
use crate::TypeTable;
use alloc::{boxed::Box, collections::BTreeMap, string::String, vec, vec::Vec};
use prod_ir::{Alt, Definition, Expr, Type};

pub(crate) type LocalTypes = BTreeMap<String, Type>;

pub(crate) fn expression_type(
    expr: &Expr,
    definitions: &[Definition],
    table: &TypeTable<'_>,
    locals: &LocalTypes,
    parameters: &[(String, Type)],
) -> Option<Type> {
    match expr {
        Expr::Var(name) => locals.get(name).cloned(),
        Expr::Param(index) => parameters.get(*index).map(|(_, ty)| ty.clone()),
        Expr::String(_) | Expr::Join(..) | Expr::FormatDecimal(..) => Some(Type::String),
        Expr::Bytes(_) | Expr::Utf8Encode(..) => Some(Type::Bytes),
        Expr::Append(left, _) => expression_type(left, definitions, table, locals, parameters),
        Expr::Utf8Decode(_) => Some(Type::Option(Box::new(Type::String))),
        Expr::SplitExact(..) => Some(Type::Option(Box::new(Type::List(Box::new(Type::String))))),
        Expr::Slice(value, ..) => {
            match expression_type(value, definitions, table, locals, parameters)? {
                ty @ (Type::Bytes | Type::List(_)) => Some(Type::Option(Box::new(ty))),
                _ => None,
            }
        }
        Expr::Index(value, _) => {
            match expression_type(value, definitions, table, locals, parameters)? {
                Type::Bytes => Some(Type::Option(Box::new(Type::UInt8))),
                Type::List(element) => Some(Type::Option(element)),
                _ => None,
            }
        }
        Expr::Call(name, _) => definitions
            .iter()
            .find(|def| def.name == *name)
            .map(|def| def.ret.clone()),
        Expr::Proj(owner, field, _) => table.get(owner.as_str()).and_then(|decl| {
            decl.ctors
                .iter()
                .flat_map(|ctor| &ctor.fields)
                .find(|(name, _)| name == field)
                .map(|(_, ty)| ty.clone())
        }),
        Expr::ParseDecimalAs(ty, _) => Some(Type::Option(Box::new(ty.clone()))),
        Expr::Let(name, value, body) => {
            let mut nested = locals.clone();
            if let Some(ty) = expression_type(value, definitions, table, locals, parameters) {
                nested.insert(name.clone(), ty);
            } else {
                nested.remove(name);
            }
            expression_type(body, definitions, table, &nested, parameters)
        }
        Expr::If(_, yes, no) => {
            let yes = expression_type(yes, definitions, table, locals, parameters)?;
            (expression_type(no, definitions, table, locals, parameters)? == yes).then_some(yes)
        }
        Expr::Ctor(name, args) if name == "Option.some" && args.len() == 1 => {
            expression_type(&args[0], definitions, table, locals, parameters)
                .map(|ty| Type::Option(Box::new(ty)))
        }
        Expr::Ctor(name, args) if name == "Prod.mk" => args
            .iter()
            .map(|arg| expression_type(arg, definitions, table, locals, parameters))
            .collect::<Option<Vec<_>>>()
            .map(Type::Tuple),
        Expr::Ctor(name, _) => table
            .values()
            .find(|decl| decl.ctors.iter().any(|ctor| ctor.name == *name))
            .map(|decl| Type::Named(decl.name.clone())),
        _ => None,
    }
}

pub(crate) fn pattern_types(
    scrutinee: Option<&Type>,
    alt: &Alt,
    table: &TypeTable<'_>,
) -> LocalTypes {
    let fields = match (scrutinee, alt.ctor.as_str()) {
        (Some(Type::Option(value)), "Option.some") => vec![(**value).clone()],
        (Some(Type::Result { ok, .. }), "Except.ok") => vec![(**ok).clone()],
        (Some(Type::Result { error, .. }), "Except.error") => vec![(**error).clone()],
        (Some(Type::List(element)), "List.cons") => {
            vec![(**element).clone(), Type::List(element.clone())]
        }
        (Some(Type::Tuple(fields)), "Prod.mk") => fields.clone(),
        (Some(Type::Named(owner)), name) => table
            .get(owner.as_str())
            .and_then(|decl| decl.ctors.iter().find(|ctor| ctor.name == name))
            .map(|ctor| ctor.fields.iter().map(|(_, ty)| ty.clone()).collect())
            .unwrap_or_default(),
        _ => Vec::new(),
    };
    if fields.len() != alt.binders.len() {
        return LocalTypes::new();
    }
    alt.binders.iter().cloned().zip(fields).collect()
}
