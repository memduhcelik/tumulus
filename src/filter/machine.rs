//! Evaluation on a heap stack.
//!
//! A function that calls itself a million times cannot sit on the process
//! stack. Pipes, tests, arithmetic, and calls wait here instead.

use std::rc::Rc;

use crate::value::Value;

use super::ast::*;
use super::eval::{self, fail, fresh, truthy};
use super::func::{self, ScopeNode};
use super::stream::LimitHow;
use super::value::{apply_op, cmp_value, negate_value, sub_values};

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
    /// `[]`, `{}`, `//`, `try`, `reduce`, `as`, `first`, `limit`, `map`, and
    /// the index after an array. The child runs on this stack.
    Nest(Box<Nest>),
    /// `..` and `recurse` with no arguments, once a call is already here.
    /// One value leaves at a time, so `limit` can stop.
    Dots(Dots),
    /// `recurse(f)` and `recurse(f; cond)`. One value leaves at a time.
    Recur(Recur),
    /// `walk(f)`. Children are rebuilt here, then `f` runs.
    Walk(Walk),
    /// `inputs`. One value is read each time this frame is stepped.
    Inputs,
    MatchRun(MatchRun),
    StreamRun(StreamRun),
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
    /// 0 while the condition runs, 1 while its branch runs.
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

struct Nest {
    kind: Kind,
    input: Value,
    env: Rc<func::EnvLink>,
    scope: Rc<ScopeNode>,
    phase: u8,
    started: bool,
    /// The child was stopped after `first` or a full `limit` count.
    stopped_child: bool,
    got: Vec<Value>,
    err: Option<Failure>,
    out: Vec<Value>,
    out_at: usize,
    emit_now: bool,
    fail_last: Option<Failure>,
    buf: Vec<Value>,
    items: Vec<Value>,
    item_at: usize,
    acc: Value,
    /// Key positions for the accumulator object while `reduce` writes in place.
    lookup: Option<super::update::ObjIndex>,
    frames: Vec<Vec<(String, Value)>>,
    frame_at: usize,
    alts: Vec<eval::Alts>,
    keys: Vec<Value>,
    key_err: Option<Failure>,
    take: Take,
    seen: usize,
    /// Set while a `label` frame is on the stack. Dropped with the frame.
    label_guard: Option<eval::LabelGuard>,
    label_runtime: Option<u32>,
}

enum Kind {
    Collect(Rc<Filter>),
    At {
        base: Rc<Filter>,
        key: Rc<Filter>,
        optional: bool,
        bases: Vec<Value>,
    },
    Alt {
        left: Rc<Filter>,
        right: Rc<Filter>,
    },
    Try {
        body: Rc<Filter>,
        handler: Option<Rc<Filter>>,
    },
    Bind {
        expr: Rc<Filter>,
        pattern: Pattern,
        body: Rc<Filter>,
    },
    /// `label $name | body`. `runtime` is this entry's `__jq` number.
    Label {
        slot: u32,
        body: Rc<Filter>,
    },
    Reduce {
        pattern: Pattern,
        init: Rc<Filter>,
        stream: Rc<Filter>,
        update: Rc<Filter>,
        inits: Vec<Value>,
    },
    Map(Rc<Filter>),
    First(Rc<Filter>),
    Limit {
        count: Rc<Filter>,
        expr: Rc<Filter>,
    },
    Object(Vec<ObjField>),
}

enum Take {
    Off,
    All,
    Count(usize),
    Sub(Value),
}

struct Dots {
    stack: Vec<Value>,
}

struct Recur {
    step: Rc<Filter>,
    cond: Option<Rc<Filter>>,
    env: Vec<(String, Value)>,
    scope: Rc<ScopeNode>,
    frames: Vec<RecurFrame>,
}

struct RecurFrame {
    value: Value,
    emitted: bool,
    step_i: usize,
    cond_i: usize,
    child: Option<Value>,
}

struct Walk {
    update: Rc<Filter>,
    env: Rc<func::EnvLink>,
    scope: Rc<ScopeNode>,
    input: Value,
    /// `None` is an array element. `Some` is an object field, in order.
    kids: Vec<(Option<String>, Value)>,
    kid_at: usize,
    built_arr: Vec<Value>,
    built_obj: Vec<(String, Value)>,
    is_object: bool,
    /// 0 not opened, 1 walking children, 2 running `f`.
    mode: u8,
    kept: bool,
    /// An object field keeps one value and drops the rest of that walk.
    cut_child: bool,
}

/// One regex/flags pair at a time. The flags filter stays put while every
/// regex value for that flag is searched.
struct MatchRun {
    regex: Rc<Filter>,
    flags: Rc<Filter>,
    testing: bool,
    input: Value,
    env: Rc<func::EnvLink>,
    scope: Rc<ScopeNode>,
    /// 0 start the flags filter, 1 reading flags, 2 reading a regex.
    phase: u8,
    flag: Option<Value>,
    started: bool,
}

/// One value of `expr`, then `body`. The next value is read only after `body`
/// finishes and the consumer still wants more.
struct StreamRun {
    expr: Rc<Filter>,
    name: String,
    body: Rc<Filter>,
    input: Value,
    env: Rc<func::EnvLink>,
    scope: Rc<ScopeNode>,
    /// 0 start `expr`, 1 reading `expr`, 2 running `body`.
    phase: u8,
    started: bool,
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
                match give(machine, parent, value, emit) {
                    Up::Go(true) => return Ok(true),
                    Up::Go(false) => {}
                    // A frame rejected the value. That frame fails the way a
                    // child failure does, so `try` inside a function can catch it.
                    Up::Fail { index, failure } => {
                        let parent = machine.stack[index as usize].parent;
                        machine.stack.truncate(index as usize);
                        if let Some(stopped) = failed(machine, parent, failure, emit)? {
                            return Ok(stopped);
                        }
                    }
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
        Body::Nest(nest) => Some(&nest.scope),
        Body::Dots(_) | Body::Inputs => None,
        Body::Recur(recur) => Some(&recur.scope),
        Body::Walk(walk) => Some(&walk.scope),
        Body::MatchRun(run) => Some(&run.scope),
        Body::StreamRun(run) => Some(&run.scope),
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
        Body::Nest(_) => step_nest(machine, index),
        Body::Dots(_) => step_dots(machine, index),
        Body::Recur(_) => step_recur(machine, index),
        Body::Walk(_) => step_walk(machine, index),
        Body::Inputs => step_inputs(machine),
        Body::MatchRun(_) => step_match_run(machine, index),
        Body::StreamRun(_) => step_stream_run(machine, index),
    }
}

fn step_inputs(_machine: &mut Machine) -> Result<Act, Failure> {
    match crate::feed::read_one() {
        crate::feed::Read::Value(value) => Ok(Act::Value(value)),
        crate::feed::Read::Break => Ok(Act::Done),
        crate::feed::Read::Parse(message) | crate::feed::Read::Fatal(message) => {
            Ok(Act::Fail(fail(message)))
        }
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
        Filter::Var { name, .. } => match env.get(name) {
            Some(value) => send_once(machine, index, value.clone()),
            None => Ok(Act::Fail(fail(format!("unbound {name}")))),
        },
        Filter::Pipe(left, right) => {
            *machine.stack[index].body = Body::Pipe(Pipe {
                left: Rc::new((**left).clone()),
                right: Rc::new((**right).clone()),
                input,
                env,
                scope,
                on_right: false,
                started: false,
            });
            Ok(Act::Cont)
        }
        Filter::Comma(left, right) => {
            *machine.stack[index].body = Body::Comma(Comma {
                left: Rc::new((**left).clone()),
                right: Rc::new((**right).clone()),
                input,
                env,
                scope,
                phase: 0,
            });
            Ok(Act::Cont)
        }
        Filter::Bin { op, left, right } => {
            *machine.stack[index].body = Body::Bin(Bin {
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
            });
            Ok(Act::Cont)
        }
        Filter::Neg(inner) => {
            *machine.stack[index].body = Body::Neg(Neg {
                inner: Rc::new((**inner).clone()),
                input,
                env,
                scope,
                started: false,
            });
            Ok(Act::Cont)
        }
        Filter::If { cond, yes, no } => {
            *machine.stack[index].body = Body::If(Ifs {
                cond: Rc::new((**cond).clone()),
                yes: Rc::new((**yes).clone()),
                no: Rc::new((**no).clone()),
                input,
                env,
                scope,
                phase: 0,
                started: false,
            });
            Ok(Act::Cont)
        }
        Filter::Call { func, args } => begin_call(machine, index, func, args, input, env, scope),
        Filter::Param(name) => begin_param(machine, index, name, input),
        Filter::Define { rest, .. } => {
            machine.kept.push(Rc::clone(&filter));
            *machine.stack[index].body = Body::Run(Run {
                filter: Rc::new((**rest).clone()),
                input,
                env,
                scope,
                sent: false,
                pending: Vec::new(),
                pending_at: 0,
                fail_after: None,
            });
            Ok(Act::Cont)
        }
        Filter::Unresolved { name, arity, .. } => {
            Ok(Act::Fail(fail(format!("{name}/{arity} is not defined"))))
        }
        Filter::Inputs => {
            *machine.stack[index].body = Body::Inputs;
            Ok(Act::Cont)
        }
        Filter::Collect(inner) => replace_nest(
            machine,
            index,
            Kind::Collect(Rc::new((**inner).clone())),
            input,
            env,
            scope,
        ),
        Filter::At {
            base,
            key,
            optional,
        } => replace_nest(
            machine,
            index,
            Kind::At {
                base: Rc::new((**base).clone()),
                key: Rc::new((**key).clone()),
                optional: *optional,
                bases: Vec::new(),
            },
            input,
            env,
            scope,
        ),
        Filter::Alt(left, right) => replace_nest(
            machine,
            index,
            Kind::Alt {
                left: Rc::new((**left).clone()),
                right: Rc::new((**right).clone()),
            },
            input,
            env,
            scope,
        ),
        Filter::Try { body, handler } => replace_nest(
            machine,
            index,
            Kind::Try {
                body: Rc::new((**body).clone()),
                handler: handler.as_ref().map(|handler| Rc::new((**handler).clone())),
            },
            input,
            env,
            scope,
        ),
        Filter::Bind { alts, .. } if !alts.is_empty() => {
            fallback(machine, index, &filter, &input, &env)
        }
        Filter::Reduce { alts, .. } if !alts.is_empty() => {
            fallback(machine, index, &filter, &input, &env)
        }
        Filter::Label { slot, body } => replace_nest(
            machine,
            index,
            Kind::Label {
                slot: *slot,
                body: Rc::new((**body).clone()),
            },
            input,
            env,
            scope,
        ),
        Filter::Bind {
            expr,
            pattern,
            body,
            ..
        } => replace_nest(
            machine,
            index,
            Kind::Bind {
                expr: Rc::new((**expr).clone()),
                pattern: pattern.clone(),
                body: Rc::new((**body).clone()),
            },
            input,
            env,
            scope,
        ),
        Filter::Reduce {
            stream,
            pattern,
            init,
            update,
            ..
        } => replace_nest(
            machine,
            index,
            Kind::Reduce {
                pattern: pattern.clone(),
                init: Rc::new((**init).clone()),
                stream: Rc::new((**stream).clone()),
                update: Rc::new((**update).clone()),
                inits: Vec::new(),
            },
            input,
            env,
            scope,
        ),
        Filter::Map(inner) => replace_nest(
            machine,
            index,
            Kind::Map(Rc::new((**inner).clone())),
            input,
            env,
            scope,
        ),
        Filter::First(Some(expr)) => replace_nest(
            machine,
            index,
            Kind::First(Rc::new((**expr).clone())),
            input,
            env,
            scope,
        ),
        Filter::Limit(count, expr) => replace_nest(
            machine,
            index,
            Kind::Limit {
                count: Rc::new((**count).clone()),
                expr: Rc::new((**expr).clone()),
            },
            input,
            env,
            scope,
        ),
        Filter::Object(fields) => replace_nest(
            machine,
            index,
            Kind::Object(fields.clone()),
            input,
            env,
            scope,
        ),
        Filter::Recurse => {
            *machine.stack[index].body = Body::Dots(Dots { stack: vec![input] });
            Ok(Act::Cont)
        }
        Filter::RecurseDown { step, cond } => {
            let cond = cond.as_ref().map(|cond| Rc::new((**cond).clone()));
            let step = Rc::new((**step).clone());
            *machine.stack[index].body = Body::Recur(Recur {
                step,
                cond,
                env: env.flatten(),
                scope,
                frames: vec![RecurFrame {
                    value: input,
                    emitted: false,
                    step_i: 0,
                    cond_i: 0,
                    child: None,
                }],
            });
            Ok(Act::Cont)
        }
        Filter::Walk(update) => {
            *machine.stack[index].body = Body::Walk(Walk {
                update: Rc::new((**update).clone()),
                env,
                scope,
                input,
                kids: Vec::new(),
                kid_at: 0,
                built_arr: Vec::new(),
                built_obj: Vec::new(),
                is_object: false,
                mode: 0,
                kept: false,
                cut_child: false,
            });
            Ok(Act::Cont)
        }
        Filter::MatchImpl {
            regex,
            flags,
            testing,
        } => {
            *machine.stack[index].body = Body::MatchRun(MatchRun {
                regex: Rc::new((**regex).clone()),
                flags: Rc::new((**flags).clone()),
                testing: *testing,
                input,
                env,
                scope,
                phase: 0,
                flag: None,
                started: false,
            });
            Ok(Act::Cont)
        }
        Filter::StreamBind { expr, name, body } => {
            *machine.stack[index].body = Body::StreamRun(StreamRun {
                expr: Rc::new((**expr).clone()),
                name: name.clone(),
                body: Rc::new((**body).clone()),
                input,
                env,
                scope,
                phase: 0,
                started: false,
            });
            Ok(Act::Cont)
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
    *machine.stack[index].body = Body::Call(Call {
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
    });
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
    *machine.stack[index].body = Body::Run(Run {
        filter: Rc::new(closure.expr.clone()),
        input,
        env: Rc::clone(&closure.env),
        scope: Rc::clone(&closure.scope),
        sent: false,
        pending: Vec::new(),
        pending_at: 0,
        fail_after: None,
    });
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

fn step_match_run(machine: &mut Machine, index: usize) -> Result<Act, Failure> {
    let started = match &*machine.stack[index].body {
        Body::MatchRun(run) => run.started,
        _ => true,
    };
    if started {
        return Ok(Act::Cont);
    }
    let (flags, input, env, scope) = match &*machine.stack[index].body {
        Body::MatchRun(run) => (
            Rc::clone(&run.flags),
            run.input.clone(),
            Rc::clone(&run.env),
            Rc::clone(&run.scope),
        ),
        _ => unreachable!("match"),
    };
    if let Body::MatchRun(run) = &mut *machine.stack[index].body {
        run.started = true;
        run.phase = 1;
    }
    push_run(machine, index, flags, input, env, scope);
    Ok(Act::Cont)
}

fn step_stream_run(machine: &mut Machine, index: usize) -> Result<Act, Failure> {
    let started = match &*machine.stack[index].body {
        Body::StreamRun(run) => run.started,
        _ => true,
    };
    if started {
        return Ok(Act::Cont);
    }
    let (expr, input, env, scope) = match &*machine.stack[index].body {
        Body::StreamRun(run) => (
            Rc::clone(&run.expr),
            run.input.clone(),
            Rc::clone(&run.env),
            Rc::clone(&run.scope),
        ),
        _ => unreachable!("stream bind"),
    };
    if let Body::StreamRun(run) = &mut *machine.stack[index].body {
        run.started = true;
        run.phase = 1;
    }
    push_run(machine, index, expr, input, env, scope);
    Ok(Act::Cont)
}

fn accept_match(machine: &mut Machine, index: usize, value: Value) -> Result<Taken, Failure> {
    let phase = match &*machine.stack[index].body {
        Body::MatchRun(run) => run.phase,
        _ => 0,
    };
    if phase == 1 {
        let (regex, input, env, scope) = match &*machine.stack[index].body {
            Body::MatchRun(run) => (
                Rc::clone(&run.regex),
                run.input.clone(),
                Rc::clone(&run.env),
                Rc::clone(&run.scope),
            ),
            _ => unreachable!("match"),
        };
        if let Body::MatchRun(run) = &mut *machine.stack[index].body {
            run.flag = Some(value);
            run.phase = 2;
        }
        push_run(machine, index, regex, input, env, scope);
        return Ok(Taken::Spawn);
    }
    let (testing, flag, input) = match &*machine.stack[index].body {
        Body::MatchRun(run) => (
            run.testing,
            run.flag.clone().unwrap_or(Value::Null),
            run.input.clone(),
        ),
        _ => unreachable!("match"),
    };
    let produced = super::re::apply_pair(&input, &value, &flag, testing)?;
    Ok(Taken::Send(produced))
}

fn accept_stream(machine: &mut Machine, index: usize, value: Value) -> Result<Taken, Failure> {
    let phase = match &*machine.stack[index].body {
        Body::StreamRun(run) => run.phase,
        _ => 0,
    };
    if phase != 1 {
        return Ok(Taken::Send(value));
    }
    let (name, body, input, env, scope) = match &*machine.stack[index].body {
        Body::StreamRun(run) => (
            run.name.clone(),
            Rc::clone(&run.body),
            run.input.clone(),
            Rc::clone(&run.env),
            Rc::clone(&run.scope),
        ),
        _ => unreachable!("stream bind"),
    };
    if let Body::StreamRun(run) = &mut *machine.stack[index].body {
        run.phase = 2;
    }
    let env = func::EnvLink::child(vec![(name, value)], &env);
    push_run(machine, index, body, input, env, scope);
    Ok(Taken::Spawn)
}

fn done_match(machine: &mut Machine, index: usize) -> Result<DoneAct, Failure> {
    let phase = match &*machine.stack[index].body {
        Body::MatchRun(run) => run.phase,
        _ => 1,
    };
    if phase == 2 {
        if let Body::MatchRun(run) = &mut *machine.stack[index].body {
            run.phase = 1;
            run.flag = None;
        }
        return Ok(DoneAct::Stay);
    }
    Ok(DoneAct::Finish)
}

fn done_stream(machine: &mut Machine, index: usize) -> Result<DoneAct, Failure> {
    let phase = match &*machine.stack[index].body {
        Body::StreamRun(run) => run.phase,
        _ => 1,
    };
    if phase == 2 {
        if let Body::StreamRun(run) = &mut *machine.stack[index].body {
            run.phase = 1;
        }
        return Ok(DoneAct::Stay);
    }
    Ok(DoneAct::Finish)
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

enum Up {
    Go(bool),
    /// `index` is the frame whose accept failed.
    Fail {
        index: u32,
        failure: Failure,
    },
}

fn give(
    machine: &mut Machine,
    mut parent: u32,
    mut value: Value,
    emit: &mut dyn FnMut(Value) -> bool,
) -> Up {
    loop {
        if parent == ROOT {
            return Up::Go(emit(value));
        }
        let index = parent as usize;
        match accept(machine, index, value) {
            Ok(Taken::Keep) => return Up::Go(false),
            Ok(Taken::Send(next)) => {
                parent = machine.stack[index].parent;
                value = next;
            }
            Ok(Taken::Spawn) => return Up::Go(false),
            Ok(Taken::Cut(next)) => {
                let next_parent = machine.stack[index].parent;
                machine.stack.truncate(index + 1);
                parent = next_parent;
                value = next;
            }
            Err(failure) => {
                return Up::Fail {
                    index: index as u32,
                    failure,
                }
            }
        }
    }
}

enum Taken {
    Keep,
    Send(Value),
    Spawn,
    /// Emit `value` and drop every frame above the one that accepted it.
    Cut(Value),
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
                ifs.phase = 1;
                let filter = if truthy(&value) {
                    Rc::clone(&ifs.yes)
                } else {
                    Rc::clone(&ifs.no)
                };
                let input = ifs.input.clone();
                let env = Rc::clone(&ifs.env);
                let scope = Rc::clone(&ifs.scope);
                push_run(machine, index, filter, input, env, scope);
                Ok(Taken::Spawn)
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
        Body::Run(_) | Body::Dots(_) | Body::Recur(_) | Body::Inputs => Ok(Taken::Send(value)),
        Body::Nest(_) => nest_accept(machine, index, value),
        Body::Walk(_) => walk_accept(machine, index, value),
        Body::MatchRun(_) => accept_match(machine, index, value),
        Body::StreamRun(_) => accept_stream(machine, index, value),
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
        Body::Nest(_) => nest_child_done(machine, index),
        Body::Run(_) | Body::Dots(_) | Body::Recur(_) | Body::Inputs => Ok(DoneAct::Finish),
        Body::Walk(_) => walk_done(machine, index),
        Body::MatchRun(_) => done_match(machine, index),
        Body::StreamRun(_) => done_stream(machine, index),
    }
}

fn finish_if(machine: &mut Machine, index: usize) -> Result<DoneAct, Failure> {
    let phase = match &*machine.stack[index].body {
        Body::If(ifs) => ifs.phase,
        _ => return Ok(DoneAct::Finish),
    };
    if phase == 1 {
        // The branch for this condition value is done. The condition continues.
        if let Body::If(ifs) = &mut *machine.stack[index].body {
            ifs.phase = 0;
        }
        return Ok(DoneAct::Stay);
    }
    Ok(DoneAct::Finish)
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
    /// A call is still reading a `$` argument.
    CallPull,
    /// `[]`, `try`, `//`, and the other forms that own their child.
    Nest,
    Drop,
}

fn halt_kind(machine: &Machine, index: usize) -> Halt {
    match &*machine.stack[index].body {
        Body::Bin(bin) if !bin.on_left => Halt::BinRight,
        Body::Call(call) if call.pulling => Halt::CallPull,
        Body::Nest(_) => Halt::Nest,
        Body::MatchRun(_) | Body::StreamRun(_) => Halt::Drop,
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
        stop: failure.stop.take(),
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
            Halt::Nest => match nest_on_error(machine, index, &mut failure)? {
                NestErr::Stay => return Ok(None),
                NestErr::Go(next_failure) => {
                    let next = machine.stack[index].parent;
                    machine.stack.truncate(index);
                    failure = next_failure;
                    parent = next;
                }
                NestErr::Done => {
                    // The nest turned the failure into success. Keep going
                    // the way a finished child does, so a comma after a
                    // caught break still runs.
                    let next = machine.stack[index].parent;
                    machine.stack.truncate(index);
                    if next == ROOT {
                        return Ok(Some(false));
                    }
                    if finished(machine, next, emit)? {
                        return Ok(Some(true));
                    }
                    return Ok(None);
                }
            },
            Halt::Drop => {
                let next = machine.stack[index].parent;
                machine.stack.truncate(index);
                parent = next;
            }
        }
    }
}

enum NestErr {
    Stay,
    Go(Failure),
    Done,
}

fn replace_nest(
    machine: &mut Machine,
    index: usize,
    kind: Kind,
    input: Value,
    env: Rc<func::EnvLink>,
    scope: Rc<ScopeNode>,
) -> Result<Act, Failure> {
    *machine.stack[index].body = Body::Nest(Box::new(Nest {
        kind,
        input,
        env,
        scope,
        phase: 0,
        started: false,
        stopped_child: false,
        got: Vec::new(),
        err: None,
        out: Vec::new(),
        out_at: 0,
        emit_now: false,
        fail_last: None,
        buf: Vec::new(),
        items: Vec::new(),
        item_at: 0,
        acc: Value::Null,
        lookup: None,
        frames: Vec::new(),
        frame_at: 0,
        alts: Vec::new(),
        keys: Vec::new(),
        key_err: None,
        take: Take::Off,
        seen: 0,
        label_guard: None,
        label_runtime: None,
    }));
    Ok(Act::Cont)
}

fn step_nest(machine: &mut Machine, index: usize) -> Result<Act, Failure> {
    let started = match &*machine.stack[index].body {
        Body::Nest(nest) => nest.started,
        _ => false,
    };
    if !started {
        if let Body::Nest(nest) = &mut *machine.stack[index].body {
            nest.started = true;
        }
        return nest_begin(machine, index);
    }
    let stopped = match &*machine.stack[index].body {
        Body::Nest(nest) => nest.stopped_child,
        _ => false,
    };
    if stopped {
        if let Body::Nest(nest) = &mut *machine.stack[index].body {
            nest.stopped_child = false;
        }
        return match nest_after_stop(machine, index)? {
            DoneAct::Stay => Ok(Act::Cont),
            DoneAct::Finish => Ok(Act::Done),
            DoneAct::Fail(failure) => Ok(Act::Fail(failure)),
        };
    }
    let emit_now = match &*machine.stack[index].body {
        Body::Nest(nest) => nest.emit_now,
        _ => false,
    };
    if emit_now {
        return nest_emit(machine, index);
    }
    Ok(Act::Cont)
}

fn nest_emit(machine: &mut Machine, index: usize) -> Result<Act, Failure> {
    let ready = match &*machine.stack[index].body {
        Body::Nest(nest) => nest.out_at < nest.out.len(),
        _ => false,
    };
    if ready {
        let value = match &mut *machine.stack[index].body {
            Body::Nest(nest) => {
                let value = nest.out[nest.out_at].clone();
                nest.out_at += 1;
                value
            }
            _ => unreachable!("nest"),
        };
        return Ok(Act::Value(value));
    }
    if let Body::Nest(nest) = &mut *machine.stack[index].body {
        nest.emit_now = false;
        nest.out.clear();
        nest.out_at = 0;
    }
    let failure = match &mut *machine.stack[index].body {
        Body::Nest(nest) => nest.fail_last.take(),
        _ => None,
    };
    match failure {
        Some(failure) => Ok(Act::Fail(failure)),
        None => Ok(Act::Done),
    }
}

fn nest_accept(machine: &mut Machine, index: usize, value: Value) -> Result<Taken, Failure> {
    let (phase, limit_expr, first, bind_body) = match &*machine.stack[index].body {
        Body::Nest(nest) => (
            nest.phase,
            matches!(nest.kind, Kind::Limit { .. }) && nest.phase == 1,
            matches!(nest.kind, Kind::First(_)),
            matches!(nest.kind, Kind::Bind { .. }) && nest.phase == 1,
        ),
        _ => (0, false, false, false),
    };
    if first {
        if let Body::Nest(nest) = &mut *machine.stack[index].body {
            nest.stopped_child = true;
        }
        return Ok(Taken::Cut(value));
    }
    let label = match &*machine.stack[index].body {
        Body::Nest(nest) => matches!(nest.kind, Kind::Label { .. }),
        _ => false,
    };
    if label {
        return Ok(Taken::Send(value));
    }
    if bind_body {
        return Ok(Taken::Send(value));
    }
    if limit_expr {
        return limit_take(machine, index, value);
    }
    let _ = phase;
    if let Body::Nest(nest) = &mut *machine.stack[index].body {
        nest.got.push(value);
    }
    Ok(Taken::Keep)
}

fn limit_take(machine: &mut Machine, index: usize, value: Value) -> Result<Taken, Failure> {
    let take = match &mut *machine.stack[index].body {
        Body::Nest(nest) => match &mut nest.take {
            Take::All => {
                return Ok(Taken::Send(value));
            }
            Take::Count(need) => {
                nest.seen += 1;
                let stop = nest.seen >= *need;
                if stop {
                    nest.stopped_child = true;
                }
                return Ok(if stop {
                    Taken::Cut(value)
                } else {
                    Taken::Send(value)
                });
            }
            Take::Sub(current) => current.clone(),
            Take::Off => return Ok(Taken::Send(value)),
        },
        _ => return Ok(Taken::Send(value)),
    };
    let one = Value::Number(crate::value::Text::from("1".to_string()));
    let next = match sub_values(&take, &one) {
        Ok(next) => next,
        Err(message) => return Err(fresh(Vec::new(), message)),
    };
    let zero = Value::Number(crate::value::Text::from("0".to_string()));
    let stop = cmp_value(&next, &zero) <= 0;
    if let Body::Nest(nest) = &mut *machine.stack[index].body {
        nest.take = Take::Sub(next);
        if stop {
            nest.stopped_child = true;
        }
    }
    Ok(if stop {
        Taken::Cut(value)
    } else {
        Taken::Send(value)
    })
}

fn nest_arm(machine: &mut Machine, index: usize, filter: Rc<Filter>, input: Value) {
    let env = match &*machine.stack[index].body {
        Body::Nest(nest) => Rc::clone(&nest.env),
        _ => unreachable!("nest"),
    };
    nest_arm_in(machine, index, filter, input, env);
}

fn nest_arm_in(
    machine: &mut Machine,
    index: usize,
    filter: Rc<Filter>,
    input: Value,
    env: Rc<func::EnvLink>,
) {
    let scope = match &*machine.stack[index].body {
        Body::Nest(nest) => Rc::clone(&nest.scope),
        _ => unreachable!("nest"),
    };
    if let Body::Nest(nest) = &mut *machine.stack[index].body {
        nest.got.clear();
        nest.err = None;
    }
    push_run(machine, index, filter, input, env, scope);
}

fn nest_begin(machine: &mut Machine, index: usize) -> Result<Act, Failure> {
    let kind = match &*machine.stack[index].body {
        Body::Nest(nest) => kind_tag(&nest.kind),
        _ => 0,
    };
    let input = match &*machine.stack[index].body {
        Body::Nest(nest) => nest.input.clone(),
        _ => Value::Null,
    };
    match kind {
        1 => {
            let inner = match &*machine.stack[index].body {
                Body::Nest(nest) => match &**nest {
Nest {
                    kind: Kind::Collect(inner),
                    ..
                } => Rc::clone(inner),
_ => unreachable!("collect"),
},
_ => unreachable!("collect"),
            };
            nest_arm(machine, index, inner, input);
        }
        2 => {
            let base = match &*machine.stack[index].body {
                Body::Nest(nest) => match &**nest {
Nest {
                    kind: Kind::At { base, .. },
                    ..
                } => Rc::clone(base),
_ => unreachable!("at"),
},
_ => unreachable!("at"),
            };
            nest_arm(machine, index, base, input);
        }
        3 => {
            let left = match &*machine.stack[index].body {
                Body::Nest(nest) => match &**nest {
Nest {
                    kind: Kind::Alt { left, .. },
                    ..
                } => Rc::clone(left),
_ => unreachable!("alt"),
},
_ => unreachable!("alt"),
            };
            nest_arm(machine, index, left, input);
        }
        4 => {
            let body = match &*machine.stack[index].body {
                Body::Nest(nest) => match &**nest {
Nest {
                    kind: Kind::Try { body, .. },
                    ..
                } => Rc::clone(body),
_ => unreachable!("try"),
},
_ => unreachable!("try"),
            };
            nest_arm(machine, index, body, input);
        }
        5 => {
            let expr = match &*machine.stack[index].body {
                Body::Nest(nest) => match &**nest {
Nest {
                    kind: Kind::Bind { expr, .. },
                    ..
                } => Rc::clone(expr),
_ => unreachable!("bind"),
},
_ => unreachable!("bind"),
            };
            nest_arm(machine, index, expr, input);
        }
        6 => {
            let init = match &*machine.stack[index].body {
                Body::Nest(nest) => match &**nest {
Nest {
                    kind: Kind::Reduce { init, .. },
                    ..
                } => Rc::clone(init),
_ => unreachable!("reduce"),
},
_ => unreachable!("reduce"),
            };
            nest_arm(machine, index, init, input);
        }
        7 => {
            let items = match &*machine.stack[index].body {
                Body::Nest(nest) => eval::iterate(&nest.input),
                _ => unreachable!("map"),
            };
            match items {
                Err(failure) => return Ok(Act::Fail(failure)),
                Ok(items) if items.is_empty() => {
                    return hold(machine, index, vec![Value::array(Vec::new())], None);
                }
                Ok(items) => {
                    let inner = match &*machine.stack[index].body {
                        Body::Nest(nest) => match &**nest {
Nest {
                            kind: Kind::Map(inner),
                            ..
                        } => Rc::clone(inner),
_ => unreachable!("map"),
},
_ => unreachable!("map"),
                    };
                    let first = items[0].clone();
                    if let Body::Nest(nest) = &mut *machine.stack[index].body {
                        nest.items = items;
                        nest.item_at = 0;
                    }
                    nest_arm(machine, index, inner, first);
                }
            }
        }
        8 => {
            let expr = match &*machine.stack[index].body {
                Body::Nest(nest) => match &**nest {
Nest {
                    kind: Kind::First(expr),
                    ..
                } => Rc::clone(expr),
_ => unreachable!("first"),
},
_ => unreachable!("first"),
            };
            nest_arm(machine, index, expr, input);
        }
        9 => {
            let count = match &*machine.stack[index].body {
                Body::Nest(nest) => match &**nest {
Nest {
                    kind: Kind::Limit { count, .. },
                    ..
                } => Rc::clone(count),
_ => unreachable!("limit"),
},
_ => unreachable!("limit"),
            };
            nest_arm(machine, index, count, input);
        }
        11 => {
            let (slot, body) = match &*machine.stack[index].body {
                Body::Nest(nest) => match &**nest {
Nest {
                    kind: Kind::Label { slot, body },
                    ..
                } => (*slot, Rc::clone(body)),
_ => unreachable!("label"),
},
_ => unreachable!("label"),
            };
            let (runtime, guard) = eval::LabelGuard::enter(slot);
            if let Body::Nest(nest) = &mut *machine.stack[index].body {
                nest.label_runtime = Some(runtime);
                nest.label_guard = Some(guard);
            }
            nest_arm(machine, index, body, input);
        }
        _ => {
            let empty = match &*machine.stack[index].body {
                Body::Nest(nest) => match &**nest {
Nest {
                    kind: Kind::Object(fields),
                    ..
                } => fields.is_empty(),
_ => true,
},
_ => true,
            };
            if empty {
                return hold(machine, index, vec![Value::object(Vec::new())], None);
            }
            let key = object_key(machine, index);
            nest_arm(machine, index, key, input);
        }
    }
    Ok(Act::Cont)
}

fn kind_tag(kind: &Kind) -> u8 {
    match kind {
        Kind::Collect(_) => 1,
        Kind::At { .. } => 2,
        Kind::Alt { .. } => 3,
        Kind::Try { .. } => 4,
        Kind::Bind { .. } => 5,
        Kind::Reduce { .. } => 6,
        Kind::Map(_) => 7,
        Kind::First(_) => 8,
        Kind::Limit { .. } => 9,
        Kind::Object(_) => 10,
        Kind::Label { .. } => 11,
    }
}

fn object_key(machine: &Machine, index: usize) -> Rc<Filter> {
    match &*machine.stack[index].body {
        Body::Nest(nest) => match &**nest {
Nest {
            kind: Kind::Object(fields),
            item_at,
            ..
        } => {
            let field = &fields[*item_at];
            let key = match field {
                ObjField::Lookup(key) | ObjField::Pair { key, .. } => key,
            };
            Rc::new((**key).clone())
        },
_ => unreachable!("object"),
},
_ => unreachable!("object"),
    }
}

fn hold(
    machine: &mut Machine,
    index: usize,
    values: Vec<Value>,
    failure: Option<Failure>,
) -> Result<Act, Failure> {
    if let Body::Nest(nest) = &mut *machine.stack[index].body {
        nest.out = values;
        nest.out_at = 0;
        nest.fail_last = failure;
        nest.emit_now = true;
    }
    nest_emit(machine, index)
}

fn nest_child_done(machine: &mut Machine, index: usize) -> Result<DoneAct, Failure> {
    let tag = match &*machine.stack[index].body {
        Body::Nest(nest) => (kind_tag(&nest.kind), nest.phase),
        _ => (0, 0),
    };
    match tag {
        (1, _) => finish_collect(machine, index),
        (2, 0) => finish_at_base(machine, index),
        (2, _) => finish_at_key(machine, index),
        (3, 0) => finish_alt_left(machine, index),
        (3, _) => finish_alt_right(machine, index),
        (4, 0) => finish_try_body(machine, index),
        (4, _) => finish_try_handler(machine, index),
        (5, 0) => finish_bind_expr(machine, index),
        (5, _) => finish_bind_body(machine, index),
        (6, 0) => finish_reduce_init(machine, index),
        (6, 1) => finish_reduce_stream(machine, index),
        (6, _) => finish_reduce_update(machine, index),
        (7, _) => finish_map_item(machine, index),
        (8, _) => finish_first(machine, index),
        (9, 0) => finish_limit_count(machine, index),
        (9, _) => finish_limit_expr(machine, index),
        (10, 0) => finish_object_key(machine, index),
        (11, _) => finish_label(machine, index),
        _ => finish_object_value(machine, index),
    }
}

fn finish_label(machine: &mut Machine, index: usize) -> Result<DoneAct, Failure> {
    let err = match &mut *machine.stack[index].body {
        Body::Nest(nest) => nest.err.take(),
        _ => None,
    };
    Ok(match err {
        Some(failure) => DoneAct::Fail(failure),
        None => DoneAct::Finish,
    })
}

fn nest_after_stop(machine: &mut Machine, index: usize) -> Result<DoneAct, Failure> {
    let limit = match &*machine.stack[index].body {
        Body::Nest(nest) => matches!(nest.kind, Kind::Limit { .. }),
        _ => false,
    };
    if limit {
        if let Body::Nest(nest) = &mut *machine.stack[index].body {
            nest.take = Take::Off;
            nest.seen = 0;
        }
        return limit_next(machine, index);
    }
    Ok(DoneAct::Finish)
}

fn finish_collect(machine: &mut Machine, index: usize) -> Result<DoneAct, Failure> {
    let (items, err) = take_got(machine, index);
    if let Some(failure) = err {
        return Ok(DoneAct::Fail(eval::bare_fail(failure)));
    }
    queue_out(machine, index, vec![Value::array(items)], None)
}

fn finish_at_base(machine: &mut Machine, index: usize) -> Result<DoneAct, Failure> {
    let (bases, err) = take_got(machine, index);
    if let Some(failure) = err {
        return Ok(DoneAct::Fail(eval::with_partial(bases, failure)));
    }
    let key = match &*machine.stack[index].body {
        Body::Nest(nest) => match &**nest {
Nest {
            kind: Kind::At { key, .. },
            ..
        } => Rc::clone(key),
_ => unreachable!("at"),
},
_ => unreachable!("at"),
    };
    let input = match &*machine.stack[index].body {
        Body::Nest(nest) => nest.input.clone(),
        _ => Value::Null,
    };
    if let Body::Nest(nest) = &mut *machine.stack[index].body {
if let Nest {
        kind: Kind::At { bases: slot, .. },
        phase,
        ..
    } = &mut **nest {

        *slot = bases;
        *phase = 1;
    }
}
    nest_arm(machine, index, key, input);
    Ok(DoneAct::Stay)
}

fn finish_at_key(machine: &mut Machine, index: usize) -> Result<DoneAct, Failure> {
    let (keys, key_err) = take_got(machine, index);
    let (bases, optional) = match &*machine.stack[index].body {
        Body::Nest(nest) => match &**nest {
Nest {
            kind: Kind::At {
                bases, optional, ..
            },
            ..
        } => (bases.clone(), *optional),
_ => (Vec::new(), false),
},
_ => (Vec::new(), false),
    };
    match index_pairs(&bases, &keys, optional, key_err) {
        Ok(values) => queue_out(machine, index, values, None),
        Err(failure) => {
            let partial = failure.partial.clone();
            queue_out(machine, index, partial, Some(eval::bare_fail(failure)))
        }
    }
}

fn index_pairs(
    bases: &[Value],
    keys: &[Value],
    optional: bool,
    key_err: Option<Failure>,
) -> Result<Vec<Value>, Failure> {
    if let Some(failure) = key_err {
        let caught = failure.caught.clone();
        let message = failure.message;
        let mut partial = Vec::new();
        for key in keys {
            for value in bases {
                match eval::index_by(value, key) {
                    Ok(got) => partial.extend(got),
                    Err(inner) => {
                        partial.extend(inner.partial);
                        return Err(Failure {
                            partial,
                            message,
                            caught,
                            stop: failure.stop.clone(),
                        });
                    }
                }
            }
        }
        return Err(Failure {
            partial,
            message,
            caught,
            stop: failure.stop,
        });
    }
    let mut out = Vec::new();
    for key in keys {
        for value in bases {
            match eval::index_by(value, key) {
                Ok(got) => out.extend(got),
                Err(mut failure) => {
                    out.extend(std::mem::take(&mut failure.partial));
                    if failure.stop.is_some() || !optional {
                        return Err(eval::with_partial(out, failure));
                    }
                }
            }
        }
    }
    Ok(out)
}

fn finish_alt_left(machine: &mut Machine, index: usize) -> Result<DoneAct, Failure> {
    let (values, err) = take_got(machine, index);
    if let Some(failure) = err {
        let kept = values.into_iter().filter(truthy).collect();
        return Ok(DoneAct::Fail(eval::with_partial(kept, failure)));
    }
    let defined: Vec<Value> = values.into_iter().filter(truthy).collect();
    if !defined.is_empty() {
        return queue_out(machine, index, defined, None);
    }
    let right = match &*machine.stack[index].body {
        Body::Nest(nest) => match &**nest {
Nest {
            kind: Kind::Alt { right, .. },
            ..
        } => Rc::clone(right),
_ => unreachable!("alt"),
},
_ => unreachable!("alt"),
    };
    let input = match &*machine.stack[index].body {
        Body::Nest(nest) => nest.input.clone(),
        _ => Value::Null,
    };
    if let Body::Nest(nest) = &mut *machine.stack[index].body {
        nest.phase = 1;
    }
    nest_arm(machine, index, right, input);
    Ok(DoneAct::Stay)
}

fn finish_alt_right(machine: &mut Machine, index: usize) -> Result<DoneAct, Failure> {
    let (values, err) = take_got(machine, index);
    match err {
        Some(failure) => Ok(DoneAct::Fail(eval::with_partial(values, failure))),
        None => queue_out(machine, index, values, None),
    }
}

fn finish_try_body(machine: &mut Machine, index: usize) -> Result<DoneAct, Failure> {
    let (partial, err) = take_got(machine, index);
    let Some(failure) = err else {
        return queue_out(machine, index, partial, None);
    };
    if failure.stop.is_some() {
        return Ok(DoneAct::Fail(eval::with_partial(partial, failure)));
    }
    let handler = match &*machine.stack[index].body {
        Body::Nest(nest) => match &**nest {
Nest {
            kind: Kind::Try { handler, .. },
            ..
        } => handler.clone(),
_ => None,
},
_ => None,
    };
    let Some(handler) = handler else {
        return queue_out(machine, index, partial, None);
    };
    let caught = match failure.caught {
        Some(value) => value,
        None => Value::String(crate::value::Text::from(failure.message)),
    };
    if let Body::Nest(nest) = &mut *machine.stack[index].body {
        nest.buf = partial;
        nest.phase = 1;
    }
    nest_arm(machine, index, handler, caught);
    Ok(DoneAct::Stay)
}

fn finish_try_handler(machine: &mut Machine, index: usize) -> Result<DoneAct, Failure> {
    let (more, err) = take_got(machine, index);
    let mut partial = match &mut *machine.stack[index].body {
        Body::Nest(nest) => std::mem::take(&mut nest.buf),
        _ => Vec::new(),
    };
    if let Some(failure) = err {
        partial.extend(more);
        return Ok(DoneAct::Fail(eval::with_partial(partial, failure)));
    }
    partial.extend(more);
    queue_out(machine, index, partial, None)
}

fn finish_bind_expr(machine: &mut Machine, index: usize) -> Result<DoneAct, Failure> {
    let (values, err) = take_got(machine, index);
    let pattern = match &*machine.stack[index].body {
        Body::Nest(nest) => match &**nest {
Nest {
            kind: Kind::Bind { pattern, .. },
            ..
        } => pattern.clone(),
_ => unreachable!("bind"),
},
_ => unreachable!("bind"),
    };
    let flat = match &*machine.stack[index].body {
        Body::Nest(nest) => nest.env.flatten(),
        _ => Vec::new(),
    };
    let mut frames = Vec::new();
    for value in &values {
        bind_frames(&pattern, value, &flat, &mut frames)?;
    }
    if let Body::Nest(nest) = &mut *machine.stack[index].body {
        nest.frames = frames;
        nest.frame_at = 0;
        nest.fail_last = err;
        nest.phase = 1;
    }
    bind_next(machine, index)
}

fn bind_frames(
    pattern: &Pattern,
    value: &Value,
    env: &[(String, Value)],
    frames: &mut Vec<Vec<(String, Value)>>,
) -> Result<(), Failure> {
    if let Pattern::Var(name) = pattern {
        frames.push(vec![(name.clone(), value.clone())]);
        return Ok(());
    }
    eval::match_pat(pattern, value, env, &[], &mut |frame| {
        frames.push(frame.to_vec());
        Ok(())
    })
}

fn bind_next(machine: &mut Machine, index: usize) -> Result<DoneAct, Failure> {
    let next = match &*machine.stack[index].body {
        Body::Nest(nest) if nest.frame_at < nest.frames.len() => {
            let frame = nest.frames[nest.frame_at].clone();
            let body = match &nest.kind {
                Kind::Bind { body, .. } => Rc::clone(body),
                _ => unreachable!("bind"),
            };
            let env = func::EnvLink::child(frame, &nest.env);
            Some((body, env, nest.input.clone()))
        }
        _ => None,
    };
    if let Some((body, env, input)) = next {
        if let Body::Nest(nest) = &mut *machine.stack[index].body {
            nest.frame_at += 1;
        }
        nest_arm_in(machine, index, body, input, env);
        return Ok(DoneAct::Stay);
    }
    let failure = match &mut *machine.stack[index].body {
        Body::Nest(nest) => nest.fail_last.take(),
        _ => None,
    };
    match failure {
        Some(failure) => Ok(DoneAct::Fail(failure)),
        None => Ok(DoneAct::Finish),
    }
}

fn finish_bind_body(machine: &mut Machine, index: usize) -> Result<DoneAct, Failure> {
    let err = match &mut *machine.stack[index].body {
        Body::Nest(nest) => nest.err.take(),
        _ => None,
    };
    if let Some(failure) = err {
        return Ok(DoneAct::Fail(failure));
    }
    bind_next(machine, index)
}

fn finish_reduce_init(machine: &mut Machine, index: usize) -> Result<DoneAct, Failure> {
    let (inits, err) = take_got(machine, index);
    if inits.is_empty() {
        return Ok(match err {
            Some(failure) => DoneAct::Fail(failure),
            None => DoneAct::Finish,
        });
    }
    if let Body::Nest(nest) = &mut *machine.stack[index].body {
if let Nest {
        kind: Kind::Reduce { inits: slot, .. },
        ..
    } = &mut **nest {

        *slot = inits;
    }
}
    if let Body::Nest(nest) = &mut *machine.stack[index].body {
        nest.fail_last = err;
        nest.seen = 0;
        nest.phase = 1;
    }
    reduce_stream(machine, index)
}

fn reduce_stream(machine: &mut Machine, index: usize) -> Result<DoneAct, Failure> {
    let (stream, input, acc) = match &*machine.stack[index].body {
        Body::Nest(nest) => {
            let init = match &nest.kind {
                Kind::Reduce { inits, stream, .. } => {
                    let value = inits.get(nest.seen).cloned().unwrap_or(Value::Null);
                    (Rc::clone(stream), value)
                }
                _ => (Rc::new(Filter::Empty), Value::Null),
            };
            let input = if nest.seen == 0 {
                nest.input.clone()
            } else {
                Value::Null
            };
            (init.0, input, init.1)
        }
        _ => (Rc::new(Filter::Empty), Value::Null, Value::Null),
    };
    if let Body::Nest(nest) = &mut *machine.stack[index].body {
        nest.acc = acc;
        nest.lookup = None;
        nest.phase = 1;
    }
    nest_arm(machine, index, stream, input);
    Ok(DoneAct::Stay)
}

fn finish_reduce_stream(machine: &mut Machine, index: usize) -> Result<DoneAct, Failure> {
    let (items, err) = take_got(machine, index);
    if let Body::Nest(nest) = &mut *machine.stack[index].body {
        nest.items = items;
        nest.item_at = 0;
        nest.frame_at = 0;
        nest.frames.clear();
        nest.key_err = err;
        nest.phase = 2;
    }
    reduce_item(machine, index)
}

enum NextItem {
    End,
    Loaded,
}

fn load_next_reduce_item(machine: &mut Machine, index: usize) -> Result<NextItem, Failure> {
    let more = match &*machine.stack[index].body {
        Body::Nest(nest) => nest.item_at < nest.items.len(),
        _ => false,
    };
    if !more {
        return Ok(NextItem::End);
    }
    let (pattern, item, flat) = match &*machine.stack[index].body {
        Body::Nest(nest) => {
            let pattern = match &nest.kind {
                Kind::Reduce { pattern, .. } => pattern.clone(),
                _ => Pattern::Var(String::new()),
            };
            let item = nest.items[nest.item_at].clone();
            (pattern, item, nest.env.flatten())
        }
        _ => (Pattern::Var(String::new()), Value::Null, Vec::new()),
    };
    let mut frames = Vec::new();
    bind_frames(&pattern, &item, &flat, &mut frames)?;
    if let Body::Nest(nest) = &mut *machine.stack[index].body {
        nest.item_at += 1;
        nest.frames = frames;
        nest.frame_at = 0;
    }
    Ok(NextItem::Loaded)
}

fn reduce_item(machine: &mut Machine, index: usize) -> Result<DoneAct, Failure> {
    match load_next_reduce_item(machine, index)? {
        NextItem::End => reduce_end_init(machine, index),
        NextItem::Loaded => reduce_frame(machine, index),
    }
}

/// One fast step. `Ok(false)` leaves the frame for the copying path.
fn accelerate_machine_frame(machine: &mut Machine, index: usize) -> Result<bool, Failure> {
    let (update, env) = {
        let Body::Nest(nest) = &*machine.stack[index].body else {
            return Ok(false);
        };
        let update = match &nest.kind {
            Kind::Reduce { update, .. } => std::rc::Rc::clone(update),
            _ => return Ok(false),
        };
        if !super::update::reduce_can_accelerate(&update) {
            return Ok(false);
        }
        let frame = nest.frames[nest.frame_at].clone();
        let outer = nest.env.flatten();
        let env = eval::child_scope(&frame, &outer);
        (update, env)
    };
    let Body::Nest(nest) = &mut *machine.stack[index].body else {
        return Ok(false);
    };
    let handled = super::update::accelerate_reduce(&update, &mut nest.acc, &env, &mut nest.lookup)?;
    if !handled {
        return Ok(false);
    }
    nest.frame_at += 1;
    nest.phase = 2;
    Ok(true)
}

fn reduce_frame(machine: &mut Machine, index: usize) -> Result<DoneAct, Failure> {
    loop {
        let ready = match &*machine.stack[index].body {
            Body::Nest(nest) => nest.frame_at < nest.frames.len(),
            _ => false,
        };
        if !ready {
            match load_next_reduce_item(machine, index)? {
                NextItem::End => return reduce_end_init(machine, index),
                NextItem::Loaded => continue,
            }
        }
        match accelerate_machine_frame(machine, index) {
            Ok(true) => continue,
            Ok(false) => {}
            Err(failure) => {
                let done = match &mut *machine.stack[index].body {
                    Body::Nest(nest) => std::mem::take(&mut nest.buf),
                    _ => Vec::new(),
                };
                return Ok(DoneAct::Fail(eval::with_partial(done, failure)));
            }
        }
        break;
    }
    let (update, env, acc) = match &*machine.stack[index].body {
        Body::Nest(nest) => {
            let frame = nest.frames[nest.frame_at].clone();
            let update = match &nest.kind {
                Kind::Reduce { update, .. } => Rc::clone(update),
                _ => Rc::new(Filter::Empty),
            };
            let env = func::EnvLink::child(frame, &nest.env);
            (update, env, nest.acc.clone())
        }
        _ => (
            Rc::new(Filter::Empty),
            func::EnvLink::from_slice(&[]),
            Value::Null,
        ),
    };
    if let Body::Nest(nest) = &mut *machine.stack[index].body {
        nest.frame_at += 1;
        nest.phase = 2;
    }
    nest_arm_in(machine, index, update, acc, env);
    Ok(DoneAct::Stay)
}

fn finish_reduce_update(machine: &mut Machine, index: usize) -> Result<DoneAct, Failure> {
    let (values, err) = take_got(machine, index);
    if let Some(failure) = err {
        let done = match &mut *machine.stack[index].body {
            Body::Nest(nest) => std::mem::take(&mut nest.buf),
            _ => Vec::new(),
        };
        return Ok(DoneAct::Fail(eval::with_partial(done, failure)));
    }
    let acc = values.last().cloned().unwrap_or(Value::Null);
    if let Body::Nest(nest) = &mut *machine.stack[index].body {
        nest.acc = acc;
        nest.lookup = None;
    }
    reduce_frame(machine, index)
}

fn reduce_end_init(machine: &mut Machine, index: usize) -> Result<DoneAct, Failure> {
    let acc = match &*machine.stack[index].body {
        Body::Nest(nest) => nest.acc.clone(),
        _ => Value::Null,
    };
    let stream_err = match &mut *machine.stack[index].body {
        Body::Nest(nest) => {
            nest.buf.push(acc);
            nest.key_err.take()
        }
        _ => None,
    };
    if let Some(failure) = stream_err {
        let done = match &mut *machine.stack[index].body {
            Body::Nest(nest) => std::mem::take(&mut nest.buf),
            _ => Vec::new(),
        };
        return Ok(DoneAct::Fail(eval::with_partial(done, failure)));
    }
    if let Body::Nest(nest) = &mut *machine.stack[index].body {
        nest.seen += 1;
    }
    let more = match &*machine.stack[index].body {
        Body::Nest(nest) => match &**nest {
Nest {
            kind: Kind::Reduce { inits, .. },
            seen,
            ..
        } => *seen < inits.len(),
_ => false,
},
_ => false,
    };
    if more {
        return reduce_stream(machine, index);
    }
    let (out, err) = match &mut *machine.stack[index].body {
        Body::Nest(nest) => (std::mem::take(&mut nest.buf), nest.fail_last.take()),
        _ => (Vec::new(), None),
    };
    queue_out(machine, index, out, err)
}

fn finish_map_item(machine: &mut Machine, index: usize) -> Result<DoneAct, Failure> {
    let (values, err) = take_got(machine, index);
    if let Some(failure) = err {
        return Ok(DoneAct::Fail(eval::bare_fail(failure)));
    }
    if let Body::Nest(nest) = &mut *machine.stack[index].body {
        nest.buf.extend(values);
        nest.item_at += 1;
    }
    let more = match &*machine.stack[index].body {
        Body::Nest(nest) => nest.item_at < nest.items.len(),
        _ => false,
    };
    if !more {
        let items = match &mut *machine.stack[index].body {
            Body::Nest(nest) => std::mem::take(&mut nest.buf),
            _ => Vec::new(),
        };
        return queue_out(machine, index, vec![Value::array(items)], None);
    }
    let (inner, input) = match &*machine.stack[index].body {
        Body::Nest(nest) => {
            let inner = match &nest.kind {
                Kind::Map(inner) => Rc::clone(inner),
                _ => unreachable!("map"),
            };
            (inner, nest.items[nest.item_at].clone())
        }
        _ => unreachable!("map"),
    };
    nest_arm(machine, index, inner, input);
    Ok(DoneAct::Stay)
}

fn finish_first(machine: &mut Machine, index: usize) -> Result<DoneAct, Failure> {
    let err = match &mut *machine.stack[index].body {
        Body::Nest(nest) => nest.err.take(),
        _ => None,
    };
    Ok(match err {
        Some(failure) => DoneAct::Fail(failure),
        None => DoneAct::Finish,
    })
}

fn finish_limit_count(machine: &mut Machine, index: usize) -> Result<DoneAct, Failure> {
    let (counts, err) = take_got(machine, index);
    if let Body::Nest(nest) = &mut *machine.stack[index].body {
        nest.items = counts;
        nest.item_at = 0;
        nest.fail_last = err;
        nest.phase = 1;
        nest.take = Take::Off;
    }
    limit_next(machine, index)
}

fn finish_limit_expr(machine: &mut Machine, index: usize) -> Result<DoneAct, Failure> {
    let err = match &mut *machine.stack[index].body {
        Body::Nest(nest) => {
            nest.take = Take::Off;
            nest.seen = 0;
            nest.err.take()
        }
        _ => None,
    };
    if let Some(failure) = err {
        return Ok(DoneAct::Fail(failure));
    }
    limit_next(machine, index)
}

fn limit_next(machine: &mut Machine, index: usize) -> Result<DoneAct, Failure> {
    loop {
        let count = match &*machine.stack[index].body {
            Body::Nest(nest) => {
                if nest.item_at >= nest.items.len() {
                    None
                } else {
                    let count = nest.items[nest.item_at].clone();
                    let expr = match &nest.kind {
                        Kind::Limit { expr, .. } => Rc::clone(expr),
                        _ => Rc::new(Filter::Empty),
                    };
                    Some((count, expr, nest.input.clone()))
                }
            }
            _ => None,
        };
        let Some((count, expr, input)) = count else {
            let failure = match &mut *machine.stack[index].body {
                Body::Nest(nest) => nest.fail_last.take(),
                _ => None,
            };
            return Ok(match failure {
                Some(failure) => DoneAct::Fail(failure),
                None => DoneAct::Finish,
            });
        };
        if let Body::Nest(nest) = &mut *machine.stack[index].body {
            nest.item_at += 1;
            nest.seen = 0;
        }
        match super::stream::limit_how(&count) {
            LimitHow::None => continue,
            LimitHow::All => {
                if let Body::Nest(nest) = &mut *machine.stack[index].body {
                    nest.take = Take::All;
                }
                nest_arm(machine, index, expr, input);
                return Ok(DoneAct::Stay);
            }
            LimitHow::Count(need) => {
                if let Body::Nest(nest) = &mut *machine.stack[index].body {
                    nest.take = Take::Count(need);
                }
                nest_arm(machine, index, expr, input);
                return Ok(DoneAct::Stay);
            }
            LimitHow::Subtract => {
                if let Body::Nest(nest) = &mut *machine.stack[index].body {
                    nest.take = Take::Sub(count);
                }
                nest_arm(machine, index, expr, input);
                return Ok(DoneAct::Stay);
            }
        }
    }
}

fn finish_object_key(machine: &mut Machine, index: usize) -> Result<DoneAct, Failure> {
    let (keys, err) = take_got(machine, index);
    let is_lookup = match &*machine.stack[index].body {
        Body::Nest(nest) => match &**nest {
Nest {
            kind: Kind::Object(fields),
            item_at,
            ..
        } => {
            matches!(fields.get(*item_at), Some(ObjField::Lookup(_)))
        },
_ => false,
},
_ => false,
    };
    if is_lookup {
        let input = match &*machine.stack[index].body {
            Body::Nest(nest) => nest.input.clone(),
            _ => Value::Null,
        };
        let alt = lookup_alt(&keys, err, &input);
        if let Body::Nest(nest) = &mut *machine.stack[index].body {
            nest.alts.push(alt);
        }
        return object_next(machine, index);
    }
    let input = match &*machine.stack[index].body {
        Body::Nest(nest) => nest.input.clone(),
        _ => Value::Null,
    };
    let value = match &*machine.stack[index].body {
        Body::Nest(nest) => match &**nest {
Nest {
            kind: Kind::Object(fields),
            item_at,
            ..
        } => match &fields[*item_at] {
            ObjField::Pair { value, .. } => Rc::new((**value).clone()),
            ObjField::Lookup(_) => unreachable!("lookup"),
        },
_ => unreachable!("object"),
},
_ => unreachable!("object"),
    };
    if let Body::Nest(nest) = &mut *machine.stack[index].body {
        nest.keys = keys;
        nest.key_err = err;
        nest.phase = 1;
    }
    nest_arm(machine, index, value, input);
    Ok(DoneAct::Stay)
}

fn finish_object_value(machine: &mut Machine, index: usize) -> Result<DoneAct, Failure> {
    let (values, value_err) = take_got(machine, index);
    let (keys, key_err) = match &mut *machine.stack[index].body {
        Body::Nest(nest) => (std::mem::take(&mut nest.keys), nest.key_err.take()),
        _ => (Vec::new(), None),
    };
    let alt = pair_alt(&keys, key_err, &values, value_err);
    if let Body::Nest(nest) = &mut *machine.stack[index].body {
        nest.alts.push(alt);
    }
    object_next(machine, index)
}

fn object_next(machine: &mut Machine, index: usize) -> Result<DoneAct, Failure> {
    if let Body::Nest(nest) = &mut *machine.stack[index].body {
        nest.item_at += 1;
        nest.phase = 0;
    }
    let more = match &*machine.stack[index].body {
        Body::Nest(nest) => match &**nest {
Nest {
            kind: Kind::Object(fields),
            item_at,
            ..
        } => *item_at < fields.len(),
_ => false,
},
_ => false,
    };
    if !more {
        let alts = match &mut *machine.stack[index].body {
            Body::Nest(nest) => std::mem::take(&mut nest.alts),
            _ => Vec::new(),
        };
        return match eval::combine_fields(&alts, 0, Vec::new()) {
            Ok(values) => queue_out(machine, index, values, None),
            Err(mut failure) => {
                let partial = std::mem::take(&mut failure.partial);
                queue_out(machine, index, partial, Some(failure))
            }
        };
    }
    let key = object_key(machine, index);
    let input = match &*machine.stack[index].body {
        Body::Nest(nest) => nest.input.clone(),
        _ => Value::Null,
    };
    nest_arm(machine, index, key, input);
    Ok(DoneAct::Stay)
}

fn lookup_alt(keys: &[Value], key_err: Option<Failure>, input: &Value) -> eval::Alts {
    let mut pairs = Vec::new();
    for value in keys {
        let Value::String(name) = value else {
            return eval::Alts {
                pairs,
                error: Some(fail(super::parse::key_message(value))),
            };
        };
        match eval::index_by(input, &Value::String(name.clone())) {
            Ok(got) => {
                let one = got.into_iter().next().unwrap_or(Value::Null);
                pairs.push((name.to_string(), one));
            }
            Err(failure) => {
                return eval::Alts {
                    pairs,
                    error: Some(eval::bare_fail(failure)),
                };
            }
        }
    }
    eval::Alts {
        pairs,
        error: key_err,
    }
}

fn pair_alt(
    keys: &[Value],
    key_err: Option<Failure>,
    values: &[Value],
    value_err: Option<Failure>,
) -> eval::Alts {
    let mut pairs = Vec::new();
    for key in keys {
        let Value::String(name) = key else {
            return eval::Alts {
                pairs,
                error: Some(fail(super::parse::key_message(key))),
            };
        };
        for item in values {
            pairs.push((name.to_string(), item.clone()));
        }
        if let Some(failure) = &value_err {
            return eval::Alts {
                pairs,
                error: Some(failure.clone()),
            };
        }
    }
    eval::Alts {
        pairs,
        error: key_err.or(value_err),
    }
}

fn take_got(machine: &mut Machine, index: usize) -> (Vec<Value>, Option<Failure>) {
    match &mut *machine.stack[index].body {
        Body::Nest(nest) => (std::mem::take(&mut nest.got), nest.err.take()),
        _ => (Vec::new(), None),
    }
}

fn queue_out(
    machine: &mut Machine,
    index: usize,
    values: Vec<Value>,
    failure: Option<Failure>,
) -> Result<DoneAct, Failure> {
    if values.is_empty() {
        return Ok(match failure {
            Some(failure) => DoneAct::Fail(failure),
            None => DoneAct::Finish,
        });
    }
    if let Body::Nest(nest) = &mut *machine.stack[index].body {
        nest.out = values;
        nest.out_at = 0;
        nest.fail_last = failure;
        nest.emit_now = true;
    }
    Ok(DoneAct::Stay)
}

fn nest_on_error(
    machine: &mut Machine,
    index: usize,
    failure: &mut Failure,
) -> Result<NestErr, Failure> {
    let caught_here = match &*machine.stack[index].body {
        Body::Nest(nest) => match (&nest.kind, nest.label_runtime) {
            (Kind::Label { .. }, Some(runtime)) => eval::is_label_hit(failure, runtime),
            _ => false,
        },
        _ => false,
    };
    if caught_here {
        return Ok(NestErr::Done);
    }
    if let Body::Nest(nest) = &mut *machine.stack[index].body {
        if nest.err.is_none() {
            nest.err = Some(note(failure));
        }
    }
    match nest_child_done(machine, index)? {
        DoneAct::Stay => Ok(NestErr::Stay),
        DoneAct::Fail(failure) => Ok(NestErr::Go(failure)),
        DoneAct::Finish => Ok(NestErr::Done),
    }
}

fn step_dots(machine: &mut Machine, index: usize) -> Result<Act, Failure> {
    let value = match &mut *machine.stack[index].body {
        Body::Dots(dots) => dots.stack.pop(),
        _ => None,
    };
    let Some(value) = value else {
        return Ok(Act::Done);
    };
    if let Ok(mut children) = eval::iterate(&value) {
        children.reverse();
        if let Body::Dots(dots) = &mut *machine.stack[index].body {
            dots.stack.extend(children);
        }
    }
    Ok(Act::Value(value))
}

/// Emit the frame, then enter the next output of `step`. With no `cond`,
/// that output is entered once. With `cond`, once per truthy result.
/// The current value is emitted before `cond` runs. A later output of `step`
/// or `cond` is not run until this branch is finished, so `limit` can stop
/// an endless walk before an error sitting beside it.
fn step_recur(machine: &mut Machine, index: usize) -> Result<Act, Failure> {
    let Body::Recur(recur) = &mut *machine.stack[index].body else {
        return Ok(Act::Done);
    };
    match recur_pull(recur) {
        Ok(Some(value)) => Ok(Act::Value(value)),
        Ok(None) => Ok(Act::Done),
        Err(failure) => Ok(Act::Fail(failure)),
    }
}

fn recur_pull(recur: &mut Recur) -> Result<Option<Value>, Failure> {
    loop {
        if recur.frames.is_empty() {
            return Ok(None);
        }
        let emitted = recur.frames.last().unwrap().emitted;
        if !emitted {
            recur.frames.last_mut().unwrap().emitted = true;
            let value = recur.frames.last().unwrap().value.clone();
            return Ok(Some(value));
        }
        let has_child = recur.frames.last().unwrap().child.is_some();
        if !has_child {
            let value = recur.frames.last().unwrap().value.clone();
            let step_i = recur.frames.last().unwrap().step_i;
            let step = Rc::clone(&recur.step);
            match super::stream::nth_value(&step, &value, &recur.env, step_i)? {
                None => {
                    recur.frames.pop();
                }
                Some(child) => {
                    let frame = recur.frames.last_mut().unwrap();
                    frame.child = Some(child);
                    frame.cond_i = 0;
                }
            }
            continue;
        }
        let child = recur.frames.last().unwrap().child.clone().unwrap();
        let cond_i = recur.frames.last().unwrap().cond_i;
        let descend = if let Some(cond) = recur.cond.clone() {
            super::stream::nth_truthy(&cond, &child, &recur.env, cond_i)?
        } else {
            cond_i == 0
        };
        if descend {
            recur.frames.last_mut().unwrap().cond_i += 1;
            let child = recur.frames.last().unwrap().child.clone().unwrap();
            recur.frames.push(RecurFrame {
                value: child,
                emitted: false,
                step_i: 0,
                cond_i: 0,
                child: None,
            });
        } else {
            let frame = recur.frames.last_mut().unwrap();
            frame.child = None;
            frame.step_i += 1;
        }
    }
}

fn step_walk(machine: &mut Machine, index: usize) -> Result<Act, Failure> {
    let (mode, cut) = match &*machine.stack[index].body {
        Body::Walk(walk) => (walk.mode, walk.cut_child),
        _ => return Ok(Act::Cont),
    };
    if mode == 0 {
        walk_open(machine, index);
        return Ok(Act::Cont);
    }
    if !cut {
        return Ok(Act::Cont);
    }
    if let Body::Walk(walk) = &mut *machine.stack[index].body {
        walk.cut_child = false;
    }
    match walk_done(machine, index)? {
        DoneAct::Stay => Ok(Act::Cont),
        DoneAct::Finish => Ok(Act::Done),
        DoneAct::Fail(failure) => Ok(Act::Fail(failure)),
    }
}

fn walk_open(machine: &mut Machine, index: usize) {
    let input = match &mut *machine.stack[index].body {
        Body::Walk(walk) => std::mem::replace(&mut walk.input, Value::Null),
        _ => return,
    };
    let input = match input.into_array() {
        Ok(items) => {
            let kids = items.into_iter().map(|value| (None, value)).collect();
            walk_set_kids(machine, index, false, kids);
            return;
        }
        Err(input) => input,
    };
    let input = match input.into_object() {
        Ok(pairs) => {
            let kids = pairs
                .into_iter()
                .map(|(key, value)| (Some(key), value))
                .collect();
            walk_set_kids(machine, index, true, kids);
            return;
        }
        Err(input) => input,
    };
    walk_apply(machine, index, input);
}

fn walk_set_kids(
    machine: &mut Machine,
    index: usize,
    is_object: bool,
    kids: Vec<(Option<String>, Value)>,
) {
    let empty = kids.is_empty();
    if let Body::Walk(walk) = &mut *machine.stack[index].body {
        walk.is_object = is_object;
        walk.kids = kids;
        walk.kid_at = 0;
        walk.mode = 1;
        walk.kept = false;
    }
    if empty {
        let value = if is_object {
            Value::object(Vec::new())
        } else {
            Value::array(Vec::new())
        };
        walk_apply(machine, index, value);
        return;
    }
    walk_push_child(machine, index);
}

fn walk_apply(machine: &mut Machine, index: usize, value: Value) {
    let (update, env, scope) = match &*machine.stack[index].body {
        Body::Walk(walk) => (
            Rc::clone(&walk.update),
            Rc::clone(&walk.env),
            Rc::clone(&walk.scope),
        ),
        _ => return,
    };
    if let Body::Walk(walk) = &mut *machine.stack[index].body {
        walk.mode = 2;
    }
    push_run(machine, index, update, value, env, scope);
}

fn walk_push_child(machine: &mut Machine, index: usize) {
    let (update, env, scope, input) = match &*machine.stack[index].body {
        Body::Walk(walk) => {
            let input = walk.kids[walk.kid_at].1.clone();
            (
                Rc::clone(&walk.update),
                Rc::clone(&walk.env),
                Rc::clone(&walk.scope),
                input,
            )
        }
        _ => return,
    };
    machine.stack.push(Item {
        parent: index as u32,
        body: Box::new(Body::Walk(Walk {
            update,
            env,
            scope,
            input,
            kids: Vec::new(),
            kid_at: 0,
            built_arr: Vec::new(),
            built_obj: Vec::new(),
            is_object: false,
            mode: 0,
            kept: false,
            cut_child: false,
        })),
    });
}

/// An array keeps every output of a child. An object keeps the first and
/// drops a field whose walk yields nothing. Outputs of `f` go upward.
fn walk_accept(machine: &mut Machine, index: usize, value: Value) -> Result<Taken, Failure> {
    let mode = match &*machine.stack[index].body {
        Body::Walk(walk) => walk.mode,
        _ => return Ok(Taken::Send(value)),
    };
    if mode == 2 {
        return Ok(Taken::Send(value));
    }
    let mut cut = false;
    if let Body::Walk(walk) = &mut *machine.stack[index].body {
        if walk.is_object {
            if !walk.kept {
                let key = walk
                    .kids
                    .get(walk.kid_at)
                    .and_then(|(key, _)| key.clone())
                    .unwrap_or_default();
                walk.built_obj.push((key, value));
                walk.kept = true;
                // The rest of this field is not run. An error or an endless
                // `recurse` there never starts.
                walk.cut_child = true;
                cut = true;
            }
        } else {
            walk.built_arr.push(value);
        }
    }
    if cut {
        machine.stack.truncate(index + 1);
    }
    Ok(Taken::Keep)
}

fn walk_done(machine: &mut Machine, index: usize) -> Result<DoneAct, Failure> {
    let mode = match &*machine.stack[index].body {
        Body::Walk(walk) => walk.mode,
        _ => return Ok(DoneAct::Finish),
    };
    if mode != 1 {
        return Ok(DoneAct::Finish);
    }
    let kid_at = match &mut *machine.stack[index].body {
        Body::Walk(walk) => {
            walk.kid_at += 1;
            walk.kept = false;
            walk.kid_at
        }
        _ => 0,
    };
    let more = match &*machine.stack[index].body {
        Body::Walk(walk) => kid_at < walk.kids.len(),
        _ => false,
    };
    if more {
        walk_push_child(machine, index);
        return Ok(DoneAct::Stay);
    }
    let value = match &mut *machine.stack[index].body {
        Body::Walk(walk) => {
            if walk.is_object {
                Value::object(std::mem::take(&mut walk.built_obj))
            } else {
                Value::array(std::mem::take(&mut walk.built_arr))
            }
        }
        _ => Value::Null,
    };
    walk_apply(machine, index, value);
    Ok(DoneAct::Stay)
}
