//! Specialize acyclic continuations before name-indexed ownership analysis.
//! The same join parameter may borrow at one call site and own at another.

use crate::{Error, JpContext};
use alloc::{
    boxed::Box,
    collections::{BTreeMap, BTreeSet},
    string::String,
    vec::Vec,
};
use prod_ir::{Definition, Expr};

// Compiler resource limits, not application/runtime capacities. Check the
// expanded size before copying a continuation, including indirect DAG reuse.
const MAX_NODES: usize = 65_536;
const MAX_DEPTH: usize = 128;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Cost {
    nodes: usize,
    depth: usize,
}

impl Cost {
    fn add(self, other: Self) -> Result<Self, Error> {
        let result = Self {
            nodes: self
                .nodes
                .checked_add(other.nodes)
                .ok_or(Error::JoinExpansionLimit)?,
            depth: self.depth.max(other.depth),
        };
        result.require(MAX_NODES, MAX_DEPTH)
    }

    fn require(self, nodes: usize, depth: usize) -> Result<Self, Error> {
        if self.nodes > nodes || self.depth > depth {
            Err(Error::JoinExpansionLimit)
        } else {
            Ok(self)
        }
    }
}

fn cost(
    source: &Expr,
    context: &JpContext<'_>,
    memo: &mut BTreeMap<String, Cost>,
    active: &mut BTreeSet<String>,
) -> Result<Cost, Error> {
    match source {
        Expr::Jmp(name, arguments) if context.decls.contains_key(name.as_str()) => {
            let (parameters, body) = context.decls[name.as_str()];
            if parameters.len() != arguments.len() || active.contains(name) {
                return Err(Error::UnsupportedJoinPoint(name.clone()));
            }
            if active.len() >= MAX_DEPTH {
                return Err(Error::JoinExpansionLimit);
            }
            let body_cost = if let Some(value) = memo.get(name) {
                *value
            } else {
                active.insert(name.clone());
                let value = cost(body, context, memo, active)?;
                active.remove(name);
                memo.insert(name.clone(), value);
                value
            };
            let mut result = Cost {
                nodes: body_cost.nodes,
                depth: body_cost
                    .depth
                    .checked_add(1)
                    .ok_or(Error::JoinExpansionLimit)?,
            }
            .add(Cost {
                nodes: parameters.len(),
                depth: 0,
            })?;
            for argument in arguments {
                result = result.add(cost(argument, context, memo, active)?)?;
            }
            Ok(result)
        }
        Expr::Jp { name, body, .. } if context.jmp_count(name) == 0 => {
            cost(body, context, memo, active)
        }
        Expr::Jp { .. } => Ok(Cost { nodes: 1, depth: 0 }),
        _ => {
            let mut result = Cost { nodes: 1, depth: 0 };
            for child in source.children() {
                result = result.add(cost(child, context, memo, active)?)?;
            }
            Ok(result)
        }
    }
}

/// Input names are already alpha-unique. Callers normalize again afterward,
/// because each continuation expansion creates a new lexical binding scope.
pub(crate) fn expand(definition: &Definition) -> Result<Option<Definition>, Error> {
    let context = JpContext::collect(&definition.body);
    if context.decls.is_empty() {
        return Ok(None);
    }
    for name in context.decls.keys() {
        if context.is_cyclic(name) {
            return Err(Error::UnsupportedJoinPoint(String::from(*name)));
        }
    }
    cost(
        &definition.body,
        &context,
        &mut BTreeMap::new(),
        &mut BTreeSet::new(),
    )?;
    let mut result = definition.clone();
    expression(&mut result.body, &context, &mut BTreeSet::new())?;
    Ok(Some(result))
}

fn expression(
    source: &mut Expr,
    context: &JpContext<'_>,
    active: &mut BTreeSet<String>,
) -> Result<(), Error> {
    match source {
        Expr::Jmp(name, arguments) if context.decls.contains_key(name.as_str()) => {
            let (parameters, body) = context.decls[name.as_str()];
            if parameters.len() != arguments.len() || active.contains(name) {
                return Err(Error::UnsupportedJoinPoint(name.clone()));
            }
            // Arguments are evaluated in the caller's scope, in source order.
            for argument in arguments.iter_mut() {
                expression(argument, context, active)?;
            }
            active.insert(name.clone());
            let mut expanded = body.clone();
            expression(&mut expanded, context, active)?;
            active.remove(name);
            for (parameter, argument) in parameters.iter().zip(core::mem::take(arguments)).rev() {
                expanded = Expr::Let(parameter.clone(), Box::new(argument), Box::new(expanded));
            }
            *source = expanded;
        }
        Expr::Jp { name, body, .. } => {
            if context.jmp_count(name) == 0 {
                let mut expanded = *body.clone();
                expression(&mut expanded, context, active)?;
                *source = expanded;
            } else {
                // The declaration previously rendered unit; only jump sites
                // execute its body. Retain the let binder if raw IR uses it.
                *source = Expr::Ctor(String::from("Prod.mk"), Vec::new());
            }
        }
        _ => {
            for child in source.children_mut() {
                expression(child, context, active)?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::{format, vec};
    use prod_ir::Type;

    fn chain(length: usize, duplicate: bool) -> Definition {
        let mut body = Expr::Jmp(format!("join{}", length - 1), vec![Expr::Nat(1)]);
        for index in (0..length).rev() {
            let parameter = format!("value{index}");
            let continuation = if index == 0 {
                Expr::Var(parameter.clone())
            } else {
                let jump = Expr::Jmp(
                    format!("join{}", index - 1),
                    vec![Expr::Var(parameter.clone())],
                );
                if duplicate {
                    Expr::If(
                        Box::new(Expr::Bool(true)),
                        Box::new(jump.clone()),
                        Box::new(jump),
                    )
                } else {
                    jump
                }
            };
            let name = format!("join{index}");
            body = Expr::Let(
                name.clone(),
                Box::new(Expr::Jp {
                    name,
                    params: vec![parameter],
                    body: Box::new(continuation),
                }),
                Box::new(body),
            );
        }
        Definition {
            name: String::from("bounded"),
            params: vec![],
            ret: Type::Nat,
            body,
        }
    }

    fn measured(definition: &Definition) -> Result<Cost, Error> {
        cost(
            &definition.body,
            &JpContext::collect(&definition.body),
            &mut BTreeMap::new(),
            &mut BTreeSet::new(),
        )
    }

    #[test]
    fn exact_node_and_depth_boundaries_are_inclusive_and_checked() {
        let small = chain(1, false);
        let actual = measured(&small).unwrap();
        assert_eq!(actual, Cost { nodes: 5, depth: 1 });
        assert_eq!(actual.require(5, 1), Ok(actual));
        assert_eq!(actual.require(4, 1), Err(Error::JoinExpansionLimit));
        assert_eq!(actual.require(5, 0), Err(Error::JoinExpansionLimit));
        assert!(expand(&small).unwrap().is_some());
        let nodes = Cost {
            nodes: MAX_NODES,
            depth: MAX_DEPTH,
        };
        assert_eq!(nodes.add(Cost { nodes: 0, depth: 0 }), Ok(nodes));
        assert_eq!(
            nodes.add(Cost { nodes: 1, depth: 0 }),
            Err(Error::JoinExpansionLimit)
        );
        assert_eq!(
            Cost {
                nodes: usize::MAX,
                depth: 0
            }
            .add(nodes),
            Err(Error::JoinExpansionLimit)
        );
        assert_eq!(measured(&chain(MAX_DEPTH, false)).unwrap().depth, MAX_DEPTH);
        assert!(expand(&chain(MAX_DEPTH, false)).unwrap().is_some());
        assert_eq!(
            expand(&chain(MAX_DEPTH + 1, false)),
            Err(Error::JoinExpansionLimit)
        );
    }

    #[test]
    fn actual_expansion_accepts_exact_node_limit_and_rejects_one_more() {
        let mut source = chain(1, false);
        let Expr::Let(_, _, body) = &mut source.body else {
            unreachable!()
        };
        **body = Expr::Ctor(String::from("Host"), vec![Expr::Nat(0); MAX_NODES - 3]);
        assert_eq!(measured(&source).unwrap().nodes, MAX_NODES);
        assert!(expand(&source).unwrap().is_some());
        let Expr::Let(_, _, body) = &mut source.body else {
            unreachable!()
        };
        let Expr::Ctor(_, arguments) = body.as_mut() else {
            unreachable!()
        };
        arguments.push(Expr::Nat(0));
        assert_eq!(expand(&source), Err(Error::JoinExpansionLimit));
    }

    #[test]
    fn acyclic_exponential_expansion_is_rejected_before_materialization() {
        let source = chain(16, true);
        assert_eq!(expand(&source), Err(Error::JoinExpansionLimit));
        assert_eq!(
            source,
            chain(16, true),
            "rejection does not modify source IR"
        );
    }
}
