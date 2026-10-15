//! Scope of a function call.
//!
//! A `$` parameter is a value in the environment. A filter parameter is a
//! closure: the argument expression, the caller's variables, and the caller's
//! filter parameters. The closure points at the caller, not at the scope that
//! holds it.

use std::cell::RefCell;
use std::rc::Rc;

use crate::value::Value;

use super::ast::*;
use super::eval::{child_scope, eval_in, fail};

thread_local! {
    static SCOPE: RefCell<Option<Rc<ScopeNode>>> = RefCell::new(None);
}

pub(super) struct Closure {
    pub expr: Filter,
    pub env: Vec<(String, Value)>,
    pub scope: Rc<ScopeNode>,
}

pub(super) struct ScopeNode {
    params: Vec<(String, Rc<Closure>)>,
    parent: Option<Rc<ScopeNode>>,
}

pub(super) struct Frame {
    pub env: Vec<(String, Value)>,
    pub scope: Rc<ScopeNode>,
}

pub(super) enum Slot {
    Value { name: String, expr: Filter },
    Filter { name: String, closure: Rc<Closure> },
}

pub(super) struct ScopeGuard {
    prev: Option<Rc<ScopeNode>>,
}

impl Drop for ScopeGuard {
    fn drop(&mut self) {
        let prev = self.prev.take();
        SCOPE.with(|slot| *slot.borrow_mut() = prev);
    }
}

pub(super) fn current_scope() -> Rc<ScopeNode> {
    SCOPE.with(|slot| {
        let mut scope = slot.borrow_mut();
        if scope.is_none() {
            *scope = Some(empty_scope());
        }
        Rc::clone(scope.as_ref().expect("scope"))
    })
}

pub(super) fn set_scope(scope: Rc<ScopeNode>) -> ScopeGuard {
    let prev = SCOPE.with(|slot| std::mem::replace(&mut *slot.borrow_mut(), Some(scope)));
    ScopeGuard { prev }
}

/// Install an empty scope for one top-level run. Drop restores the previous one.
pub(super) fn root_scope() -> ScopeGuard {
    set_scope(empty_scope())
}

fn empty_scope() -> Rc<ScopeNode> {
    Rc::new(ScopeNode {
        params: Vec::new(),
        parent: None,
    })
}

pub(super) fn lookup(name: &str) -> Option<Rc<Closure>> {
    fn find(scope: &ScopeNode, name: &str) -> Option<Rc<Closure>> {
        if let Some((_, closure)) = scope.params.iter().rev().find(|(bound, _)| bound == name) {
            return Some(Rc::clone(closure));
        }
        scope.parent.as_ref().and_then(|parent| find(parent, name))
    }
    SCOPE.with(|slot| {
        slot.borrow()
            .as_ref()
            .and_then(|scope| find(scope, name))
    })
}

pub(super) fn prepare(
    params: &[FnParam],
    args: &[Filter],
    env: &[(String, Value)],
    scope: &Rc<ScopeNode>,
) -> Vec<Slot> {
    params
        .iter()
        .zip(args.iter())
        .map(|(param, arg)| match param {
            FnParam::Value(name) => Slot::Value {
                name: name.clone(),
                expr: arg.clone(),
            },
            FnParam::Filter(name) => Slot::Filter {
                name: name.clone(),
                closure: Rc::new(Closure {
                    expr: arg.clone(),
                    env: env.to_vec(),
                    scope: Rc::clone(scope),
                }),
            },
        })
        .collect()
}

pub(super) fn frame(
    values: &[(String, Value)],
    filters: &[(String, Rc<Closure>)],
    caller_env: &[(String, Value)],
    caller_scope: &Rc<ScopeNode>,
) -> Frame {
    Frame {
        env: child_scope(values, caller_env),
        scope: Rc::new(ScopeNode {
            params: filters.to_vec(),
            parent: Some(Rc::clone(caller_scope)),
        }),
    }
}

pub(super) fn eval_param(
    name: &str,
    input: &Value,
    _env: &[(String, Value)],
) -> Result<Vec<Value>, Failure> {
    let Some(closure) = lookup(name) else {
        return Err(fail(format!("{name} is not defined")));
    };
    let _guard = set_scope(Rc::clone(&closure.scope));
    eval_in(&closure.expr, input, &closure.env)
}
