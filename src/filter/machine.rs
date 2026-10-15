//! Evaluation on a heap stack.
//!
//! A function that calls itself a million times cannot sit on the process
//! stack. Pipes, tests, arithmetic, and calls wait here instead.

use std::rc::Rc;

use crate::value::Value;

use super::ast::*;
use super::eval::{self, fail, fresh, truthy};
use super::func::{self, ScopeNode};
use super::value::{apply_op, negate_value};

const ROOT: u32 = u32::MAX;

struct Machine {
    stack: Vec<Item>,
    /// Keeps a `def` alive while its rest runs, so the calls in the rest
    /// still find the function.
    kept: Vec<Rc<Filter>>,
}

struct Item {
    parent: u32,
    body: Box<Body>,
}

enum Body {
    Run(Run),
    Pipe(Pipe),
    Comma(Comma),
    Bin(Bin),
    Neg(Neg),
    If(Ifs),
    Call(Call),
}

struct Run {
    filter: Rc<Filter>,
    input: Value,
    env: Rc<func::EnvLink>,
    scope: Rc<ScopeNode>,
    /// One value has been handed upward. Finish once this frame is on top
    /// and nothing in `pending` remains.
    sent: bool,
    /// Later values from one filter. They leave one at a time so a pipe can
    /// run its right-hand side between them.
    pending: Vec<Value>,
    pending_at: usize,
    /// Raised after `pending` is empty. Its partial values are already `pending`.
    fail_after: Option<Failure>,
}

struct Pipe {
    left: Rc<Filter>,
    right: Rc<Filter>,
    input: Value,
    env: Rc<func::EnvLink>,
    scope: Rc<ScopeNode>,
    /// The right-hand filter is the child that is running.
    on_right: bool,
    started: bool,
}

struct Comma {
    left: Rc<Filter>,
    right: Rc<Filter>,
    input: Value,
    env: Rc<func::EnvLink>,
    scope: Rc<ScopeNode>,
    /// 0 not started, 1 left running, 2 right running.
    phase: u8,
}

struct Bin {
    op: Op,
    left: Rc<Filter>,
    right: Rc<Filter>,
    input: Value,
    env: Rc<func::EnvLink>,
    scope: Rc<ScopeNode>,
    on_left: bool,
    rights: Vec<Value>,
    right_at: usize,
    right_err: Option<Failure>,
    started: bool,
}

struct Neg {
    inner: Rc<Filter>,
    input: Value,
    env: Rc<func::EnvLink>,
    scope: Rc<ScopeNode>,
    started: bool,
}

struct Ifs {
    cond: Rc<Filter>,
    yes: Rc<Filter>,
    no: Rc<Filter>,
    input: Value,
    env: Rc<func::EnvLink>,
    scope: Rc<ScopeNode>,
    conds: Vec<Value>,
    cond_at: usize,
    cond_err: Option<Failure>,
    /// 0 condition, 1 branch.
    phase: u8,
    started: bool,
}

struct Call {
    body: Rc<Filter>,
    input: Value,
    env: Rc<func::EnvLink>,
    caller_scope: Rc<ScopeNode>,
    slots: Vec<func::Slot>,
    filters: Vec<(String, Rc<func::Closure>)>,
    levels: Vec<Level>,
    slot_at: usize,
    pulling: bool,
    body_on: bool,
}

struct Level {
    /// Index into `slots`. Filter parameters have no level.
    slot: usize,
    name: String,
    values: Vec<Value>,
    index: usize,
    err: Option<Failure>,
}

enum Act {
    Cont,
    Value(Value),
    Done,
    Fail(Failure),
}

pub(super) fn run(
    filter: &Filter,
    input: &Value,
    env: &[(String, Value)],
    emit: &mut dyn FnMut(Value) -> bool,
) -> Result<bool, Failure> {
    let saved = func::current_scope();
    let mut machine = Machine {
        stack: vec![Item {
            parent: ROOT,
            body: Box::new(Body::Run(Run {
                filter: Rc::new(filter.clone()),
                input: input.clone(),
                env: func::EnvLink::from_slice(env),
                scope: Rc::clone(&saved),
                sent: false,
                pending: Vec::new(),
                pending_at: 0,
                fail_after: None,
            })),
        }],
        kept: Vec::new(),
    };
    let result = drive_stack(&mut machine, emit);
    func::install_scope(saved);
    result
}

fn drive_stack(
    machine: &mut Machine,
    emit: &mut dyn FnMut(Value) -> bool,
) -> Result<bool, Failure> {
    while !machine.stack.is_empty() {
        let index = machine.stack.len() - 1;
        install_top(machine, index);
        match step(machine, index)? {
            Act::Cont => {}
            Act::Value(value) => {
                let parent = machine.stack[index].parent;
                if give(machine, parent, value, emit)? {
                    return Ok(true);
                }
            }
            Act::Done => {
                let parent = machine.stack[index].parent;
                machine.stack.truncate(index);
                if parent == ROOT {
                    return Ok(false);
                }
                if finished(machine, parent, emit)? {
                    return Ok(true);
                }
            }
            Act::Fail(failure) => {
                let parent = machine.stack[index].parent;
                machine.stack.truncate(index);
                if let Some(stopped) = failed(machine, parent, failure, emit)? {
                    return Ok(stopped);
                }
            }
        }
    }
    Ok(false)
}

fn install_top(machine: &Machine, index: usize) {
    let scope = match &*machine.stack[index].body {
        Body::Run(run) => Some(&run.scope),
        Body::Pipe(pipe) => Some(&pipe.scope),
        Body::Comma(comma) => Some(&comma.scope),
        Body::Bin(bin) => Some(&bin.scope),
        Body::Neg(neg) => Some(&neg.scope),
        Body::If(ifs) => Some(&ifs.scope),
        Body::Call(call) => Some(if call.body_on {
            // The body frame carries its own scope. This frame is under it.
            &call.caller_scope
        } else {
            &call.caller_scope
        }),
    };
    if let Some(scope) = scope {
        func::install_scope(Rc::clone(scope));
    }
}

fn step(machine: &mut Machine, index: usize) -> Result<Act, Failure> {
    match &*machine.stack[index].body {
        Body::Run(_) => step_run(machine, index),
        Body::Pipe(pipe) if !pipe.started => {
            let left = Rc::clone(&pipe.left);
            let input = pipe.input.clone();
            let env = Rc::clone(&pipe.env);
            let scope = Rc::clone(&pipe.scope);
            if let Body::Pipe(pipe) = &mut *machine.stack[index].body {
                pipe.started = true;
            }
            push_run(machine, index, left, input, env, scope);
            Ok(Act::Cont)
        }
        Body::Pipe(_) => Ok(Act::Cont),
        Body::Comma(comma) if comma.phase == 0 => {
            let left = Rc::clone(&comma.left);
            let input = comma.input.clone();
            let env = Rc::clone(&comma.env);
            let scope = Rc::clone(&comma.scope);
            if let Body::Comma(comma) = &mut *machine.stack[index].body {
                comma.phase = 1;
            }
            push_run(machine, index, left, input, env, scope);
            Ok(Act::Cont)
        }
        Body::Comma(_) => Ok(Act::Cont),
        Body::Bin(bin) if !bin.started => {
            let right = Rc::clone(&bin.right);
            let input = bin.input.clone();
            let env = Rc::clone(&bin.env);
            let scope = Rc::clone(&bin.scope);
            if let Body::Bin(bin) = &mut *machine.stack[index].body {
                bin.started = true;
            }
            push_run(machine, index, right, input, env, scope);
            Ok(Act::Cont)
        }
        Body::Bin(_) => Ok(Act::Cont),
        Body::Neg(neg) if !neg.started => {
            let inner = Rc::clone(&neg.inner);
            let input = neg.input.clone();
            let env = Rc::clone(&neg.env);
            let scope = Rc::clone(&neg.scope);
            if let Body::Neg(neg) = &mut *machine.stack[index].body {
                neg.started = true;
            }
            push_run(machine, index, inner, input, env, scope);
            Ok(Act::Cont)
        }
        Body::Neg(_) => Ok(Act::Cont),
        Body::If(ifs) if !ifs.started => {
            let cond = Rc::clone(&ifs.cond);
            let input = ifs.input.clone();
            let env = Rc::clone(&ifs.env);
            let scope = Rc::clone(&ifs.scope);
            if let Body::If(ifs) = &mut *machine.stack[index].body {
                ifs.started = true;
            }
            push_run(machine, index, cond, input, env, scope);
            Ok(Act::Cont)
        }
        Body::If(_) => Ok(Act::Cont),
        Body::Call(_) => step_call(machine, index),
    }
}

fn step_run(machine: &mut Machine, index: usize) -> Result<Act, Failure> {
    let (sent, pending_left, has_fail) = match &*machine.stack[index].body {
        Body::Run(run) => (
            run.sent,
            run.pending_at < run.pending.len(),
            run.fail_after.is_some(),
        ),
        _ => unreachable!("run"),
    };
    if sent && pending_left {
        let value = match &mut *machine.stack[index].body {
            Body::Run(run) => {
                let value = run.pending[run.pending_at].clone();
                run.pending_at += 1;
                if run.pending_at == run.pending.len() {
                    run.pending.clear();
                    run.pending_at = 0;
                }
                value
            }
            _ => unreachable!("run"),
        };
        return Ok(Act::Value(value));
    }
    if sent && has_fail {
        let failure = match &mut *machine.stack[index].body {
            Body::Run(run) => run.fail_after.take().expect("fail"),
            _ => unreachable!("run"),
        };
        return Ok(Act::Fail(failure));
    }
    if sent {
        return Ok(Act::Done);
    }
    let run = match &*machine.stack[index].body {
        Body::Run(run) => run,
        _ => unreachable!("run"),
    };
    let filter = Rc::clone(&run.filter);
    let input = run.input.clone();
    let env = Rc::clone(&run.env);
    let scope = Rc::clone(&run.scope);
    match &*filter {
        Filter::Identity => send_once(machine, index, input),
        Filter::Literal(value) => send_once(machine, index, value.clone()),
        Filter::Empty => Ok(Act::Done),
        Filter::Not => send_once(machine, index, Value::Bool(!truthy(&input))),
        Filter::Var(name) => match env.get(name) {
            Some(value) => send_once(machine, index, value.clone()),
            None => Ok(Act::Fail(fail(format!("unbound {name}")))),
        },
        Filter::Pipe(left, right) => {
            machine.stack[index].body = Box::new(Body::Pipe(Pipe {
                left: Rc::new((**left).clone()),
                right: Rc::new((**right).clone()),
                input,
                env,
                scope,
                on_right: false,
                started: false,
            }));
            Ok(Act::Cont)
        }
        Filter::Comma(left, right) => {
            machine.stack[index].body = Box::new(Body::Comma(Comma {
                left: Rc::new((**left).clone()),
                right: Rc::new((**right).clone()),
                input,
                env,
                scope,
                phase: 0,
            }));
            Ok(Act::Cont)
        }
        Filter::Bin { op, left, right } => {
            machine.stack[index].body = Box::new(Body::Bin(Bin {
                op: *op,
                left: Rc::new((**left).clone()),
                right: Rc::new((**right).clone()),
                input,
                env,
                scope,
                on_left: false,
                rights: Vec::new(),
                right_at: 0,
                right_err: None,
                started: false,
            }));
            Ok(Act::Cont)
        }
        Filter::Neg(inner) => {
            machine.stack[index].body = Box::new(Body::Neg(Neg {
                inner: Rc::new((**inner).clone()),
                input,
                env,
                scope,
                started: false,
            }));
            Ok(Act::Cont)
        }
        Filter::If { cond, yes, no } => {
            machine.stack[index].body = Box::new(Body::If(Ifs {
                cond: Rc::new((**cond).clone()),
                yes: Rc::new((**yes).clone()),
                no: Rc::new((**no).clone()),
                input,
                env,
                scope,
                conds: Vec::new(),
                cond_at: 0,
                cond_err: None,
                phase: 0,
                started: false,
            }));
            Ok(Act::Cont)
        }
        Filter::Call { func, args } => begin_call(machine, index, func, args, input, env, scope),
        Filter::Param(name) => begin_param(machine, index, name, input),
        Filter::Define { rest, .. } => {
            machine.kept.push(Rc::clone(&filter));
            machine.stack[index].body = Box::new(Body::Run(Run {
                filter: Rc::new((**rest).clone()),
                input,
                env,
                scope,
                sent: false,
                pending: Vec::new(),
                pending_at: 0,
                fail_after: None,
            }));
            Ok(Act::Cont)
        }
        Filter::Unresolved { name, arity, .. } => {
            Ok(Act::Fail(fail(format!("{name}/{arity} is not defined"))))
        }
        other => fallback(machine, index, other, &input, &env),
    }
}

/// Hand the values upward one at a time. A pipe has to run its right-hand
/// side for each value, and that side is a frame above this one.
fn fallback(
    machine: &mut Machine,
    index: usize,
    filter: &Filter,
    input: &Value,
    env: &func::EnvLink,
) -> Result<Act, Failure> {
    let env = env.flatten();
    match eval::produce(filter, input, &env) {
        Ok(values) => queue_values(machine, index, values, None),
        Err(mut failure) => {
            let partial = std::mem::take(&mut failure.partial);
            if partial.is_empty() {
                Ok(Act::Fail(failure))
            } else {
                queue_values(machine, index, partial, Some(failure))
            }
        }
    }
}

fn queue_values(
    machine: &mut Machine,
    index: usize,
    mut values: Vec<Value>,
    fail_after: Option<Failure>,
) -> Result<Act, Failure> {
    if values.is_empty() {
        return Ok(match fail_after {
            Some(failure) => Act::Fail(failure),
            None => Act::Done,
        });
    }
    let first = values.remove(0);
    if let Body::Run(run) = &mut *machine.stack[index].body {
        run.sent = true;
        run.pending = values;
        run.pending_at = 0;
        run.fail_after = fail_after;
    }
    Ok(Act::Value(first))
}

fn begin_call(
    machine: &mut Machine,
    index: usize,
    func: &std::rc::Weak<FnCell>,
    args: &[Filter],
    input: Value,
    env: Rc<func::EnvLink>,
    scope: Rc<ScopeNode>,
) -> Result<Act, Failure> {
    let Some(cell) = func.upgrade() else {
        return Ok(Act::Fail(fail("function is not defined".to_string())));
    };
    let body = Rc::clone(&cell.body.borrow());
    let slots = func::prepare(&cell.params, args, &env, &scope);
    let filters: Vec<_> = slots.iter().map(func::slot_closure).collect();
    machine.stack[index].body = Box::new(Body::Call(Call {
        body,
        input,
        env,
        caller_scope: scope,
        slots,
        filters,
        levels: Vec::new(),
        slot_at: 0,
        pulling: false,
        body_on: false,
    }));
    Ok(Act::Cont)
}

fn begin_param(
    machine: &mut Machine,
    index: usize,
    name: &str,
    input: Value,
) -> Result<Act, Failure> {
    let Some(closure) = func::lookup(name) else {
        return Ok(Act::Fail(fail(format!("{name} is not defined"))));
    };
    machine.stack[index].body = Box::new(Body::Run(Run {
        filter: Rc::new(closure.expr.clone()),
        input,
        env: Rc::clone(&closure.env),
        scope: Rc::clone(&closure.scope),
        sent: false,
        pending: Vec::new(),
        pending_at: 0,
        fail_after: None,
    }));
    Ok(Act::Cont)
}

fn step_call(machine: &mut Machine, index: usize) -> Result<Act, Failure> {
    if call_body_on(machine, index) {
        // The body is a child. Wait until it finishes.
        return Ok(Act::Cont);
    }
    if call_pulling(machine, index) {
        return Ok(Act::Cont);
    }
    advance_call(machine, index)
}

fn call_body_on(machine: &Machine, index: usize) -> bool {
    match &*machine.stack[index].body {
        Body::Call(call) => call.body_on,
        _ => false,
    }
}

fn call_pulling(machine: &Machine, index: usize) -> bool {
    match &*machine.stack[index].body {
        Body::Call(call) => call.pulling,
        _ => false,
    }
}

/// Start the next `$` argument, or the body when every argument is bound.
fn advance_call(machine: &mut Machine, index: usize) -> Result<Act, Failure> {
    loop {
        let (slot_at, slot_len) = match &*machine.stack[index].body {
            Body::Call(call) => (call.slot_at, call.slots.len()),
            _ => unreachable!("call"),
        };
        if slot_at >= slot_len {
            return start_body(machine, index);
        }
        let is_value = match &*machine.stack[index].body {
            Body::Call(call) => matches!(call.slots[slot_at], func::Slot::Value { .. }),
            _ => false,
        };
        if !is_value {
            if let Body::Call(call) = &mut *machine.stack[index].body {
                call.slot_at += 1;
            }
            continue;
        }
        let (name, expr, input, env, scope) = match &*machine.stack[index].body {
            Body::Call(call) => {
                let func::Slot::Value { name, expr, .. } = &call.slots[slot_at] else {
                    unreachable!("value slot");
                };
                (
                    name.clone(),
                    expr.clone(),
                    call.input.clone(),
                    Rc::clone(&call.env),
                    Rc::clone(&call.caller_scope),
                )
            }
            _ => unreachable!("call"),
        };
        if let Body::Call(call) = &mut *machine.stack[index].body {
            call.pulling = true;
            call.levels.push(Level {
                slot: call.slot_at,
                name,
                values: Vec::new(),
                index: 0,
                err: None,
            });
        }
        push_run(machine, index, Rc::new(expr), input, env, scope);
        return Ok(Act::Cont);
    }
}

fn start_body(machine: &mut Machine, index: usize) -> Result<Act, Failure> {
    let (body, input, env, scope) = match &*machine.stack[index].body {
        Body::Call(call) => {
            let bound = bindings(call);
            let mut here = Vec::with_capacity(bound.len());
            for binding in bound.iter().rev() {
                here.push(binding.clone());
            }
            (
                Rc::clone(&call.body),
                call.input.clone(),
                func::EnvLink::child(here, &call.env),
                func::scope_under(&call.filters, &call.caller_scope),
            )
        }
        _ => unreachable!("call"),
    };
    let only = single_body(machine, index);
    if only {
        let parent = machine.stack[index].parent;
        machine.stack[index] = Item {
            parent,
            body: Box::new(Body::Run(Run {
                filter: body,
                input,
                env,
                scope,
                sent: false,
                pending: Vec::new(),
                pending_at: 0,
                fail_after: None,
            })),
        };
        return Ok(Act::Cont);
    }
    if let Body::Call(call) = &mut *machine.stack[index].body {
        call.body_on = true;
    }
    push_run(machine, index, body, input, env, scope);
    Ok(Act::Cont)
}

fn bindings(call: &Call) -> Vec<(String, Value)> {
    let mut bound = Vec::new();
    for level in &call.levels {
        if level.index < level.values.len() {
            bound.push((level.name.clone(), level.values[level.index].clone()));
        }
    }
    bound
}

/// The body about to run is the only one this call still has to run.
fn single_body(machine: &Machine, index: usize) -> bool {
    let Body::Call(call) = &*machine.stack[index].body else {
        return false;
    };
    if call.levels.iter().any(|level| level.err.is_some()) {
        return false;
    }
    call.levels
        .iter()
        .all(|level| level.values.len() <= 1 && level.index + 1 >= level.values.len())
}

fn send_once(machine: &mut Machine, index: usize, value: Value) -> Result<Act, Failure> {
    if let Body::Run(run) = &mut *machine.stack[index].body {
        run.sent = true;
    }
    Ok(Act::Value(value))
}

fn push_run(
    machine: &mut Machine,
    parent: usize,
    filter: Rc<Filter>,
    input: Value,
    env: Rc<func::EnvLink>,
    scope: Rc<ScopeNode>,
) {
    machine.stack.push(Item {
        parent: parent as u32,
        body: Box::new(Body::Run(Run {
            filter,
            input,
            env,
            scope,
            sent: false,
            pending: Vec::new(),
            pending_at: 0,
            fail_after: None,
        })),
    });
}

fn give(
    machine: &mut Machine,
    mut parent: u32,
    mut value: Value,
    emit: &mut dyn FnMut(Value) -> bool,
) -> Result<bool, Failure> {
    loop {
        if parent == ROOT {
            return Ok(emit(value));
        }
        let index = parent as usize;
        match accept(machine, index, value)? {
            Taken::Keep => return Ok(false),
            Taken::Send(next) => {
                parent = machine.stack[index].parent;
                value = next;
            }
            Taken::Spawn => return Ok(false),
        }
    }
}

enum Taken {
    Keep,
    Send(Value),
    Spawn,
}

fn accept(machine: &mut Machine, index: usize, value: Value) -> Result<Taken, Failure> {
    match &mut *machine.stack[index].body {
        Body::Pipe(pipe) => {
            if pipe.on_right {
                return Ok(Taken::Send(value));
            }
            pipe.on_right = true;
            let right = Rc::clone(&pipe.right);
            let env = Rc::clone(&pipe.env);
            let scope = Rc::clone(&pipe.scope);
            push_run(machine, index, right, value, env, scope);
            Ok(Taken::Spawn)
        }
        Body::Comma(_) => Ok(Taken::Send(value)),
        Body::Bin(bin) => {
            if !bin.on_left {
                bin.rights.push(value);
                return Ok(Taken::Keep);
            }
            let right = bin.rights.get(bin.right_at).cloned();
            let Some(right) = right else {
                return Ok(Taken::Keep);
            };
            match apply_op(bin.op, &value, &right) {
                Ok(next) => Ok(Taken::Send(next)),
                Err(message) => Err(fresh(Vec::new(), message)),
            }
        }
        Body::Neg(_) => match negate_value(&value) {
            Ok(next) => Ok(Taken::Send(next)),
            Err(message) => Err(fresh(Vec::new(), message)),
        },
        Body::If(ifs) => {
            if ifs.phase == 0 {
                ifs.conds.push(value);
                Ok(Taken::Keep)
            } else {
                Ok(Taken::Send(value))
            }
        }
        Body::Call(call) => {
            if call.pulling {
                if let Some(level) = call.levels.last_mut() {
                    level.values.push(value);
                }
                Ok(Taken::Keep)
            } else {
                Ok(Taken::Send(value))
            }
        }
        Body::Run(_) => Ok(Taken::Send(value)),
    }
}

fn finished(
    machine: &mut Machine,
    parent: u32,
    emit: &mut dyn FnMut(Value) -> bool,
) -> Result<bool, Failure> {
    let mut parent = parent;
    loop {
        if parent == ROOT {
            return Ok(false);
        }
        let index = parent as usize;
        match on_done(machine, index)? {
            DoneAct::Stay => return Ok(false),
            DoneAct::Finish => {
                let next = machine.stack[index].parent;
                machine.stack.truncate(index);
                parent = next;
            }
            DoneAct::Fail(failure) => {
                let next = machine.stack[index].parent;
                machine.stack.truncate(index);
                return match failed(machine, next, failure, emit)? {
                    Some(stopped) => Ok(stopped),
                    None => Ok(false),
                };
            }
        }
    }
}

enum DoneAct {
    Stay,
    Finish,
    Fail(Failure),
}

fn on_done(machine: &mut Machine, index: usize) -> Result<DoneAct, Failure> {
    match &mut *machine.stack[index].body {
        Body::Pipe(pipe) => {
            if pipe.on_right {
                pipe.on_right = false;
                return Ok(DoneAct::Stay);
            }
            Ok(DoneAct::Finish)
        }
        Body::Comma(comma) => {
            if comma.phase == 1 {
                comma.phase = 2;
                let right = Rc::clone(&comma.right);
                let input = comma.input.clone();
                let env = Rc::clone(&comma.env);
                let scope = Rc::clone(&comma.scope);
                push_run(machine, index, right, input, env, scope);
                return Ok(DoneAct::Stay);
            }
            Ok(DoneAct::Finish)
        }
        Body::Bin(bin) => {
            if !bin.on_left {
                bin.on_left = true;
                if bin.rights.is_empty() {
                    return Ok(match bin.right_err.take() {
                        Some(failure) => DoneAct::Fail(failure),
                        None => DoneAct::Finish,
                    });
                }
                let left = Rc::clone(&bin.left);
                let input = bin.input.clone();
                let env = Rc::clone(&bin.env);
                let scope = Rc::clone(&bin.scope);
                push_run(machine, index, left, input, env, scope);
                return Ok(DoneAct::Stay);
            }
            bin.right_at += 1;
            if bin.right_at < bin.rights.len() {
                let left = Rc::clone(&bin.left);
                let input = bin.input.clone();
                let env = Rc::clone(&bin.env);
                let scope = Rc::clone(&bin.scope);
                push_run(machine, index, left, input, env, scope);
                return Ok(DoneAct::Stay);
            }
            Ok(match bin.right_err.take() {
                Some(failure) => DoneAct::Fail(failure),
                None => DoneAct::Finish,
            })
        }
        Body::Neg(_) => Ok(DoneAct::Finish),
        Body::If(_) => finish_if(machine, index),
        Body::Call(_) => finish_call(machine, index),
        Body::Run(_) => Ok(DoneAct::Finish),
    }
}

fn finish_if(machine: &mut Machine, index: usize) -> Result<DoneAct, Failure> {
    let phase = match &*machine.stack[index].body {
        Body::If(ifs) => ifs.phase,
        _ => return Ok(DoneAct::Finish),
    };
    if phase == 0 {
        let empty = match &*machine.stack[index].body {
            Body::If(ifs) => ifs.conds.is_empty(),
            _ => true,
        };
        if empty {
            let err = match &mut *machine.stack[index].body {
                Body::If(ifs) => ifs.cond_err.take(),
                _ => None,
            };
            return Ok(match err {
                Some(failure) => DoneAct::Fail(failure),
                None => DoneAct::Finish,
            });
        }
        if let Body::If(ifs) = &mut *machine.stack[index].body {
            ifs.phase = 1;
            ifs.cond_at = 0;
        }
        push_branch(machine, index);
        return Ok(DoneAct::Stay);
    }
    let more = match &mut *machine.stack[index].body {
        Body::If(ifs) => {
            ifs.cond_at += 1;
            ifs.cond_at < ifs.conds.len()
        }
        _ => false,
    };
    if more {
        push_branch(machine, index);
        return Ok(DoneAct::Stay);
    }
    let err = match &mut *machine.stack[index].body {
        Body::If(ifs) => ifs.cond_err.take(),
        _ => None,
    };
    Ok(match err {
        Some(failure) => DoneAct::Fail(failure),
        None => DoneAct::Finish,
    })
}

fn push_branch(machine: &mut Machine, index: usize) {
    let (filter, input, env, scope) = match &*machine.stack[index].body {
        Body::If(ifs) => {
            let cond = &ifs.conds[ifs.cond_at];
            let filter = if truthy(cond) {
                Rc::clone(&ifs.yes)
            } else {
                Rc::clone(&ifs.no)
            };
            (
                filter,
                ifs.input.clone(),
                Rc::clone(&ifs.env),
                Rc::clone(&ifs.scope),
            )
        }
        _ => unreachable!("if"),
    };
    push_run(machine, index, filter, input, env, scope);
}

fn finish_call(machine: &mut Machine, index: usize) -> Result<DoneAct, Failure> {
    let pulling = call_pulling(machine, index);
    let body_on = call_body_on(machine, index);
    if pulling {
        if let Body::Call(call) = &mut *machine.stack[index].body {
            call.pulling = false;
        }
        return after_pull(machine, index);
    }
    if body_on {
        if let Body::Call(call) = &mut *machine.stack[index].body {
            call.body_on = false;
        }
        return step_levels(machine, index);
    }
    Ok(DoneAct::Finish)
}

fn after_pull(machine: &mut Machine, index: usize) -> Result<DoneAct, Failure> {
    let empty = match &*machine.stack[index].body {
        Body::Call(call) => call
            .levels
            .last()
            .is_some_and(|level| level.values.is_empty()),
        _ => false,
    };
    if empty {
        // No value, so this combination does not run. The same step that
        // follows a finished value moves to the next outer one.
        return step_levels(machine, index);
    }
    let slot = match &*machine.stack[index].body {
        Body::Call(call) => call.levels.last().map(|level| level.slot).unwrap_or(0),
        _ => 0,
    };
    if let Body::Call(call) = &mut *machine.stack[index].body {
        if let Some(level) = call.levels.last_mut() {
            level.index = 0;
        }
        call.slot_at = slot + 1;
    }
    // A single value with nothing after it replaces this frame. That happens
    // in `start_body` once the remaining slots are skipped.
    match advance_call(machine, index)? {
        Act::Cont | Act::Value(_) => Ok(DoneAct::Stay),
        Act::Done => Ok(DoneAct::Finish),
        Act::Fail(failure) => Ok(DoneAct::Fail(failure)),
    }
}

/// The value just used is finished. Take the next value of that `$` argument,
/// or the next outer one when it is used up. Later arguments are read again.
fn step_levels(machine: &mut Machine, index: usize) -> Result<DoneAct, Failure> {
    loop {
        enum Move {
            Again(usize),
            Pop,
            Done,
        }
        let action = match &mut *machine.stack[index].body {
            Body::Call(call) => {
                let Some(level) = call.levels.last_mut() else {
                    return Ok(DoneAct::Finish);
                };
                level.index += 1;
                if level.index < level.values.len() {
                    Move::Again(level.slot)
                } else {
                    Move::Pop
                }
            }
            _ => Move::Done,
        };
        match action {
            Move::Done => return Ok(DoneAct::Finish),
            Move::Pop => {
                let err = match &mut *machine.stack[index].body {
                    Body::Call(call) => call.levels.pop().and_then(|level| level.err),
                    _ => None,
                };
                if let Some(failure) = err {
                    return Ok(DoneAct::Fail(failure));
                }
            }
            Move::Again(slot) => {
                if let Body::Call(call) = &mut *machine.stack[index].body {
                    call.slot_at = slot + 1;
                    call.pulling = false;
                    call.body_on = false;
                }
                return match advance_call(machine, index)? {
                    Act::Fail(failure) => Ok(DoneAct::Fail(failure)),
                    Act::Done => Ok(DoneAct::Finish),
                    _ => Ok(DoneAct::Stay),
                };
            }
        }
    }
}

enum Halt {
    /// `+` and the other operators are still reading the right-hand side.
    BinRight,
    /// `if` is still reading the condition.
    IfCond,
    /// A call is still reading a `$` argument.
    CallPull,
    Drop,
}

fn halt_kind(machine: &Machine, index: usize) -> Halt {
    match &*machine.stack[index].body {
        Body::Bin(bin) if !bin.on_left => Halt::BinRight,
        Body::If(ifs) if ifs.phase == 0 => Halt::IfCond,
        Body::Call(call) if call.pulling => Halt::CallPull,
        _ => Halt::Drop,
    }
}

/// The values already handed upward stay with the caller. What remains is the
/// reason the filter stopped.
fn note(failure: &mut Failure) -> Failure {
    Failure {
        partial: Vec::new(),
        message: std::mem::take(&mut failure.message),
        caught: failure.caught.take(),
    }
}

fn failed(
    machine: &mut Machine,
    parent: u32,
    failure: Failure,
    emit: &mut dyn FnMut(Value) -> bool,
) -> Result<Option<bool>, Failure> {
    let mut parent = parent;
    let mut failure = failure;
    loop {
        if parent == ROOT {
            return Err(failure);
        }
        let index = parent as usize;
        match halt_kind(machine, index) {
            Halt::BinRight => {
                if let Body::Bin(bin) = &mut *machine.stack[index].body {
                    bin.right_err = Some(note(&mut failure));
                    bin.on_left = true;
                }
                let empty = match &*machine.stack[index].body {
                    Body::Bin(bin) => bin.rights.is_empty(),
                    _ => true,
                };
                if empty {
                    let err = match &mut *machine.stack[index].body {
                        Body::Bin(bin) => bin.right_err.take(),
                        _ => None,
                    };
                    let next = machine.stack[index].parent;
                    machine.stack.truncate(index);
                    failure = err.unwrap_or(failure);
                    parent = next;
                    continue;
                }
                let (left, input, env, scope) = match &*machine.stack[index].body {
                    Body::Bin(bin) => (
                        Rc::clone(&bin.left),
                        bin.input.clone(),
                        Rc::clone(&bin.env),
                        Rc::clone(&bin.scope),
                    ),
                    _ => unreachable!("bin"),
                };
                push_run(machine, index, left, input, env, scope);
                return Ok(None);
            }
            Halt::IfCond => {
                if let Body::If(ifs) = &mut *machine.stack[index].body {
                    ifs.cond_err = Some(note(&mut failure));
                }
                let empty = match &*machine.stack[index].body {
                    Body::If(ifs) => ifs.conds.is_empty(),
                    _ => true,
                };
                if empty {
                    let err = match &mut *machine.stack[index].body {
                        Body::If(ifs) => ifs.cond_err.take(),
                        _ => None,
                    };
                    let next = machine.stack[index].parent;
                    machine.stack.truncate(index);
                    failure = err.unwrap_or(failure);
                    parent = next;
                    continue;
                }
                if let Body::If(ifs) = &mut *machine.stack[index].body {
                    ifs.phase = 1;
                    ifs.cond_at = 0;
                }
                push_branch(machine, index);
                return Ok(None);
            }
            Halt::CallPull => {
                if let Body::Call(call) = &mut *machine.stack[index].body {
                    if let Some(level) = call.levels.last_mut() {
                        level.err = Some(note(&mut failure));
                    }
                    call.pulling = false;
                }
                match after_pull(machine, index)? {
                    DoneAct::Stay => return Ok(None),
                    DoneAct::Finish => {
                        let next = machine.stack[index].parent;
                        machine.stack.truncate(index);
                        if next == ROOT {
                            return Ok(Some(false));
                        }
                        return Ok(Some(finished(machine, next, emit)?));
                    }
                    DoneAct::Fail(next_failure) => {
                        let next = machine.stack[index].parent;
                        machine.stack.truncate(index);
                        failure = next_failure;
                        parent = next;
                    }
                }
            }
            Halt::Drop => {
                let next = machine.stack[index].parent;
                machine.stack.truncate(index);
                parent = next;
            }
        }
    }
}
