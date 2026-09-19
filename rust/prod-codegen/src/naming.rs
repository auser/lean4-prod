//! Keep lexical identity separate from renderer-local names. Exported LCNF is
//! already alpha-unique; public raw IR also permits shadowing and sibling reuse.
//! Normalization happens before any name-indexed ownership/inline/join analysis.
use crate::{Error, TypeTable};
use alloc::{
    collections::{BTreeMap, BTreeSet},
    format,
    string::String,
    vec::Vec,
};
use prod_ir::{Definition, Expr, Type};

type Scope = BTreeMap<String, String>;

struct Names {
    reserved: BTreeSet<String>,
    used: BTreeSet<String>,
    next: usize,
    buffer: bool,
    protected: BTreeSet<String>,
    formals: Vec<String>,
    tracked_sources: BTreeSet<String>,
    tracked_bindings: BTreeSet<String>,
}

impl Names {
    fn bind(&mut self, original: &str) -> String {
        let bound = self.bind_name(original);
        if self.tracked_sources.contains(original) {
            self.tracked_bindings.insert(bound.clone());
        }
        bound
    }

    fn bind_name(&mut self, original: &str) -> String {
        let rendered = crate::rust_local_ident(original);
        let indexed_buffer_temporary = self.buffer
            && ["__head", "__rest", "__len"].iter().any(|prefix| {
                rendered.strip_prefix(prefix).is_some_and(|suffix| {
                    !suffix.is_empty() && suffix.bytes().all(|byte| byte.is_ascii_digit())
                })
            });
        if !self.protected.contains(&rendered)
            && !indexed_buffer_temporary
            && self.used.insert(rendered)
        {
            return String::from(original);
        }
        loop {
            let candidate = format!("prod_local_{}", self.next);
            self.next += 1;
            if !self.reserved.contains(&candidate)
                && !self.protected.contains(&candidate)
                && self.used.insert(candidate.clone())
            {
                return candidate;
            }
        }
    }

    fn bindings(&mut self, bindings: &mut [String], parent: &Scope) -> Result<Scope, Error> {
        distinct_bindings(bindings.iter().map(String::as_str))?;
        let mut scope = parent.clone();
        for name in bindings {
            let old = name.clone();
            *name = self.bind(&old);
            scope.insert(old, name.clone());
        }
        Ok(scope)
    }

    fn rewrite(&mut self, expr: &mut Expr, variables: &Scope, joins: &Scope) -> Result<(), Error> {
        match expr {
            Expr::Var(name) => {
                if let Some(replacement) = variables.get(name) {
                    *name = replacement.clone();
                }
            }
            // Positional references always refer to original formals, not to
            // a same-spelled inner binder. Invalid indices retain the existing
            // ParamOutOfBounds diagnostic at rendering.
            Expr::Param(index) => {
                if let Some(name) = self.formals.get(*index) {
                    *expr = Expr::Var(name.clone());
                }
            }
            Expr::Let(name, value, body) => {
                let old = name.clone();
                if let Expr::Jp {
                    name: label,
                    params,
                    body: continuation,
                } = value.as_mut()
                {
                    // Lower.lean represents a join declaration and its lexical
                    // continuation as `(let g (jp g (...) ...) body)`.
                    let old_label = label.clone();
                    *label = self.bind(&old_label);
                    *name = if old == old_label {
                        label.clone()
                    } else {
                        self.bind(&old)
                    };
                    let mut inner_joins = joins.clone();
                    inner_joins.insert(old_label, label.clone());
                    let parameters = self.bindings(params, variables)?;
                    self.rewrite(continuation, &parameters, &inner_joins)?;
                    let mut inner_variables = variables.clone();
                    inner_variables.insert(old, name.clone());
                    self.rewrite(body, &inner_variables, &inner_joins)?;
                } else {
                    self.rewrite(value, variables, joins)?;
                    *name = self.bind(&old);
                    let mut inner = variables.clone();
                    inner.insert(old, name.clone());
                    self.rewrite(body, &inner, joins)?;
                }
            }
            Expr::Match {
                scrut,
                alts,
                default,
            } => {
                self.rewrite(scrut, variables, joins)?;
                for alt in alts {
                    let inner = self.bindings(&mut alt.binders, variables)?;
                    self.rewrite(&mut alt.body, &inner, joins)?;
                }
                if let Some(default) = default {
                    self.rewrite(default, variables, joins)?;
                }
            }
            Expr::Jp { name, params, body } => {
                let old = name.clone();
                *name = self.bind(&old);
                let mut inner_joins = joins.clone();
                inner_joins.insert(old, name.clone());
                let inner_variables = self.bindings(params, variables)?;
                self.rewrite(body, &inner_variables, &inner_joins)?;
            }
            Expr::Jmp(name, args) => {
                if let Some(replacement) = joins.get(name) {
                    *name = replacement.clone();
                }
                for arg in args {
                    self.rewrite(arg, variables, joins)?;
                }
            }
            _ => {
                for child in expr.children_mut() {
                    self.rewrite(child, variables, joins)?;
                }
            }
        }
        Ok(())
    }
}

fn distinct_bindings<'a>(names: impl Iterator<Item = &'a str>) -> Result<(), Error> {
    let mut seen = BTreeSet::new();
    for name in names {
        if !seen.insert(name) {
            return Err(Error::DuplicateBinding(String::from(name)));
        }
    }
    Ok(())
}

fn temporary_names(expr: &Expr, names: &mut BTreeSet<String>) {
    let locals: &[&str] = match expr {
        Expr::Append(..) => &["__value"],
        Expr::Index(..) => &["__index"],
        Expr::Slice(..) => &["__start", "__count", "__end", "__slice"],
        Expr::SplitExact(..) => &["__value", "__delimiter", "__limit", "__maximum", "__fields"],
        Expr::ParseDecimal(..) | Expr::ParseDecimalAs(..) => &["__input", "__text", "__value"],
        Expr::Ctor(name, _) if name == "List.cons" => &["__list"],
        _ => &[],
    };
    names.extend(locals.iter().map(|name| String::from(*name)));
    for child in expr.children() {
        temporary_names(child, names);
    }
}

fn original_names(expr: &Expr, names: &mut BTreeSet<String>) {
    let mut record = |name: &str| {
        names.insert(crate::rust_local_ident(name));
    };
    match expr {
        Expr::Var(name) | Expr::Let(name, ..) | Expr::Jmp(name, _) => record(name),
        Expr::Jp { name, params, .. } => {
            record(name);
            for name in params {
                record(name);
            }
        }
        Expr::Match { alts, .. } => {
            for alt in alts {
                for name in &alt.binders {
                    record(name);
                }
            }
        }
        _ => {}
    }
    for child in expr.children() {
        original_names(child, names);
    }
}

fn protected_names(
    expr: &Expr,
    emitted: &impl Fn(&str) -> String,
    types: &TypeTable<'_>,
    names: &mut BTreeSet<String>,
) {
    let mut constructor = |name: &str| {
        if !name.contains('.')
            && !types
                .values()
                .any(|declaration| declaration.ctors.iter().any(|ctor| ctor.name == name))
        {
            names.insert(String::from(name));
        }
    };
    match expr {
        Expr::Ctor(name, _) => constructor(name),
        Expr::Match { alts, .. } => {
            for alt in alts {
                constructor(&alt.ctor);
            }
        }
        Expr::Call(name, _) => {
            names.insert(emitted(name));
        }
        _ => {}
    }
    for child in expr.children() {
        protected_names(child, emitted, types, names);
    }
}

pub(crate) fn normalize_definition(
    definition: &Definition,
    emitted_call_name: &impl Fn(&str) -> String,
    types: &TypeTable<'_>,
) -> Result<Definition, Error> {
    normalize_tracking(definition, emitted_call_name, types, &BTreeSet::new())
        .map(|(definition, _)| definition)
}

/// Carry compiler-owned binder identity through renaming without reserving a
/// user-spellable prefix or treating coincidentally named raw IR specially.
pub(crate) fn normalize_tracking(
    definition: &Definition,
    emitted_call_name: &impl Fn(&str) -> String,
    types: &TypeTable<'_>,
    tracked_sources: &BTreeSet<String>,
) -> Result<(Definition, BTreeSet<String>), Error> {
    distinct_bindings(definition.params.iter().map(|(name, _)| name.as_str()))?;
    let mut result = definition.clone();
    let mut reserved = BTreeSet::new();
    for (name, _) in &definition.params {
        reserved.insert(crate::rust_local_ident(name));
    }
    original_names(&definition.body, &mut reserved);
    let buffer = matches!(definition.ret, Type::List(_)) && !definition.params.is_empty();
    let mut temporaries = BTreeSet::new();
    // Prelude constructors already occupy the value/pattern namespace even
    // when no constructor occurs in this body. Self and `_` cannot be bound
    // and referenced as ordinary Rust locals, but are valid raw-IR names.
    temporaries.extend(["Some", "None", "Ok", "Err", "Self", "_"].map(String::from));
    temporary_names(&definition.body, &mut temporaries);
    // Calls and Vars occupy distinct IR namespaces, but unqualified Rust
    // function/helper calls can be captured by either old or newly made locals.
    protected_names(&definition.body, emitted_call_name, types, &mut temporaries);
    let own_helper = emitted_call_name(&definition.name);
    if own_helper != definition.name {
        // The public wrapper calls its borrowed implementation even when the
        // source body contains no Call node at all.
        temporaries.insert(own_helper);
    }
    if buffer {
        temporaries.extend(["output", "__source", "__len"].map(String::from));
    }
    let mut names = Names {
        reserved,
        used: BTreeSet::new(),
        next: 0,
        buffer,
        protected: temporaries,
        formals: Vec::new(),
        tracked_sources: tracked_sources.clone(),
        tracked_bindings: BTreeSet::new(),
    };
    let mut variables = Scope::new();
    for (name, _) in &mut result.params {
        let old = name.clone();
        *name = names.bind(&old);
        variables.insert(old, name.clone());
        names.formals.push(name.clone());
    }
    names.rewrite(&mut result.body, &variables, &Scope::new())?;
    Ok((result, names.tracked_bindings))
}

#[cfg(test)]
mod tests {
    use super::*;
    use prod_ir::parser::parse_module;

    #[test]
    fn hygienic_input_is_unchanged_and_rewrite_is_idempotent() {
        let (_, module) = parse_module(
            r#"(module Names
          (def stable ((input Bytes)) Bytes
            (let g (jp g (value) value)
              (cases (utf8-decode input) (alt "Option.none" () (bytes))
                (alt "Option.some" (text) (jmp g (utf8-encode text)))))))"#,
        )
        .unwrap();
        let original = &module.definitions[0];
        let normalize = |definition: &Definition| {
            normalize_definition(definition, &|name| String::from(name), &TypeTable::new())
        };
        assert_eq!(normalize(original).unwrap(), *original);
        let (_, collision) = parse_module(
            r#"(module Names
          (def collision ((__value Bytes)) Bytes
            (let __value (bytes 2) (append __value (param 0)))))"#,
        )
        .unwrap();
        let normalized = normalize(&collision.definitions[0]).unwrap();
        assert_ne!(normalized, collision.definitions[0]);
        assert_eq!(normalize(&normalized).unwrap(), normalized);
        let (_, safe) = parse_module(
            r#"(module Names
          (def safe ((__safe Bytes)) Bytes __safe))"#,
        )
        .unwrap();
        assert_eq!(
            normalize(&safe.definitions[0]).unwrap(),
            safe.definitions[0]
        );
    }
}
