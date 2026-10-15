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

/// Bindings layered so a recursive call shares the caller's environment
/// instead of copying it.
pub(super) struct EnvLink {
    here: Vec<(String, Value)>,
    parent: Option<Rc<EnvLink>>,
}

impl EnvLink {
    pub(super) fn from_slice(env: &[(String, Value)]) -> Rc<EnvLink> {
        Rc::new(EnvLink {
            here: env.to_vec(),
            parent: None,
        })
    }

    /// `here` is searched before `parent`. The first binding of a name wins.
    pub(super) fn child(here: Vec<(String, Value)>, parent: &Rc<EnvLink>) -> Rc<EnvLink> {
        Rc::new(EnvLink {
            here,
            parent: Some(Rc::clone(parent)),
        })
    }

    pub(super) fn get(&self, name: &str) -> Option<&Value> {
        let mut node = Some(self);
        while let Some(current) = node {
            if let Some((_, value)) = current.here.iter().find(|(bound, _)| bound == name) {
                return Some(value);
            }
            node = current.parent.as_deref();
        }
        None
    }

    pub(super) fn flatten(&self) -> Vec<(String, Value)> {
        let mut out = Vec::new();
        let mut node = Some(self);
        while let Some(current) = node {
            for (name, value) in &current.here {
                if out.iter().any(|(found, _)| found == name) {
                    continue;
                }
                out.push((name.clone(), value.clone()));
            }
            node = current.parent.as_deref();
        }
        out
    }
}

impl Drop for EnvLink {
    fn drop(&mut self) {
        // A million nested calls share one parent pointer each. Dropping that
        // chain from the leaf has to stay on the heap.
        let mut next = self.parent.take();
        while let Some(node) = next {
            match Rc::try_unwrap(node) {
                Ok(mut env) => next = env.parent.take(),
                Err(shared) => {
                    drop(shared);
                    break;
                }
            }
        }
    }
}

pub(super) struct Closure {
    pub expr: Filter,
    pub env: Rc<EnvLink>,
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
    /// A `$` parameter. `expr` is read once per value. `closure` is the same
    /// argument used again wherever the name appears without `$`.
    Value {
        name: String,
        expr: Filter,
        closure: Rc<Closure>,
    },
    Filter {
        name: String,
        closure: Rc<Closure>,
    },
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

pub(super) fn install_scope(scope: Rc<ScopeNode>) {
    SCOPE.with(|slot| *slot.borrow_mut() = Some(scope));
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
    SCOPE.with(|slot| {
        let scope = slot.borrow();
        let mut node = scope.as_deref();
        while let Some(current) = node {
            if let Some((_, closure)) = current.params.iter().rev().find(|(bound, _)| bound == name)
            {
                return Some(Rc::clone(closure));
            }
            node = current.parent.as_deref();
        }
        None
    })
}

impl Drop for ScopeNode {
    fn drop(&mut self) {
        // Same shape as `EnvLink`: the caller scope is the parent, a million
        // calls deep. Detach it before the closures drop, or the leaf drop
        // walks the chain on the process stack.
        let mut next = self.parent.take();
        self.params.clear();
        while let Some(node) = next {
            match Rc::try_unwrap(node) {
                Ok(mut scope) => next = scope.parent.take(),
                Err(shared) => {
                    drop(shared);
                    break;
                }
            }
        }
    }
}

pub(super) fn prepare(
    params: &[FnParam],
    args: &[Filter],
    env: &Rc<EnvLink>,
    scope: &Rc<ScopeNode>,
) -> Vec<Slot> {
    params
        .iter()
        .zip(args.iter())
        .map(|(param, arg)| {
            let closure = Rc::new(Closure {
                expr: arg.clone(),
                env: Rc::clone(env),
                scope: Rc::clone(scope),
            });
            match param {
                FnParam::Value(name) => Slot::Value {
                    name: name.clone(),
                    expr: arg.clone(),
                    closure,
                },
                FnParam::Filter(name) => Slot::Filter {
                    name: name.clone(),
                    closure,
                },
            }
        })
        .collect()
}

pub(super) fn scope_under(
    filters: &[(String, Rc<Closure>)],
    caller_scope: &Rc<ScopeNode>,
) -> Rc<ScopeNode> {
    Rc::new(ScopeNode {
        params: filters.to_vec(),
        parent: Some(Rc::clone(caller_scope)),
    })
}

pub(super) fn frame(
    values: &[(String, Value)],
    filters: &[(String, Rc<Closure>)],
    caller_env: &[(String, Value)],
    caller_scope: &Rc<ScopeNode>,
) -> Frame {
    // The last `$` parameter of a name wins, and `child_scope` keeps the first.
    let mut ordered = Vec::with_capacity(values.len());
    for binding in values.iter().rev() {
        ordered.push(binding.clone());
    }
    Frame {
        env: child_scope(&ordered, caller_env),
        scope: Rc::new(ScopeNode {
            params: filters.to_vec(),
            parent: Some(Rc::clone(caller_scope)),
        }),
    }
}

pub(super) fn slot_closure(slot: &Slot) -> (String, Rc<Closure>) {
    match slot {
        Slot::Value { name, closure, .. } | Slot::Filter { name, closure } => {
            (name.clone(), Rc::clone(closure))
        }
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
    let env = closure.env.flatten();
    eval_in(&closure.expr, input, &env)
}
