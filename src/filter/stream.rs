//! Streaming filters.
//!
//! `limit`, `first`, and `nth` stop before a later value is run. `comma` and
//! `pipe` honor that stop, so `first(1, error("z"))` does not raise. The
//! output callback is behind a `RefCell` so `while` and `until` can keep
//! going without holding the callback across the recursive call.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use crate::number;
use crate::value::Value;

use super::ast::*;
use super::eval::{self, fail, index_by};
use super::func;
use super::value::{add_values, cmp_value, sub_values};

struct Emit<'a> {
    f: RefCell<&'a mut dyn FnMut(&Value) -> bool>,
}

impl Emit<'_> {
    fn call(&self, value: &Value) -> bool {
        (self.f.borrow_mut())(value)
    }
}

/// Hand each value to `on_value` as it is produced. `on_value` returns `true`
/// to stop. The error's partial results are values that were not handed over.
pub(super) fn feed(
    filter: &Filter,
    input: &Value,
    env: &[(String, Value)],
    on_value: &mut dyn FnMut(&Value) -> bool,
) -> Result<bool, Failure> {
    let emit = Emit {
        f: RefCell::new(on_value),
    };
    drive(filter, input, env, &emit)
}

pub(super) fn collect(
    filter: &Filter,
    input: &Value,
    env: &[(String, Value)],
) -> Result<Vec<Value>, Failure> {
    let mut out = Vec::new();
    let result = {
        let mut emit = |value: &Value| {
            out.push(value.clone());
            false
        };
        let emit = Emit {
            f: RefCell::new(&mut emit),
        };
        drive(filter, input, env, &emit)
    };
    finish_emit(result, out)
}

pub(super) fn pipe_values(
    left: &Filter,
    right: &Filter,
    input: &Value,
    env: &[(String, Value)],
) -> Result<Vec<Value>, Failure> {
    let mut out = Vec::new();
    let result = {
        let mut emit = |value: &Value| {
            out.push(value.clone());
            false
        };
        let emit = Emit {
            f: RefCell::new(&mut emit),
        };
        drive_pipe(left, right, input, env, &emit)
    };
    finish_emit(result, out)
}

pub(super) fn comma_values(
    left: &Filter,
    right: &Filter,
    input: &Value,
    env: &[(String, Value)],
) -> Result<Vec<Value>, Failure> {
    let mut out = Vec::new();
    let result = {
        let mut emit = |value: &Value| {
            out.push(value.clone());
            false
        };
        let emit = Emit {
            f: RefCell::new(&mut emit),
        };
        drive_comma(left, right, input, env, &emit)
    };
    finish_emit(result, out)
}

pub(super) fn limit_values(
    count: &Filter,
    expr: &Filter,
    input: &Value,
    env: &[(String, Value)],
) -> Result<Vec<Value>, Failure> {
    streamed(input, env, |emit| {
        drive_limit(count, expr, input, env, emit)
    })
}

pub(super) fn first_values(
    expr: Option<&Filter>,
    input: &Value,
    env: &[(String, Value)],
) -> Result<Vec<Value>, Failure> {
    streamed(input, env, |emit| drive_first(expr, input, env, emit))
}

pub(super) fn last_values(
    expr: Option<&Filter>,
    input: &Value,
    env: &[(String, Value)],
) -> Result<Vec<Value>, Failure> {
    streamed(input, env, |emit| drive_last(expr, input, env, emit))
}

pub(super) fn nth_values(
    index: &Filter,
    expr: Option<&Filter>,
    input: &Value,
    env: &[(String, Value)],
) -> Result<Vec<Value>, Failure> {
    streamed(input, env, |emit| drive_nth(index, expr, input, env, emit))
}

pub(super) fn range_values(
    from: Option<&Filter>,
    to: &Filter,
    step: Option<&Filter>,
    input: &Value,
    env: &[(String, Value)],
) -> Result<Vec<Value>, Failure> {
    streamed(input, env, |emit| {
        drive_range(from, to, step, input, env, emit)
    })
}

pub(super) fn until_values(
    cond: &Filter,
    next: &Filter,
    input: &Value,
    env: &[(String, Value)],
) -> Result<Vec<Value>, Failure> {
    streamed(input, env, |emit| drive_until(cond, next, input, env, emit))
}

pub(super) fn while_values(
    cond: &Filter,
    update: &Filter,
    input: &Value,
    env: &[(String, Value)],
) -> Result<Vec<Value>, Failure> {
    streamed(input, env, |emit| {
        drive_while(cond, update, input, env, emit)
    })
}

pub(super) fn repeat_values(
    expr: &Filter,
    input: &Value,
    env: &[(String, Value)],
) -> Result<Vec<Value>, Failure> {
    streamed(input, env, |emit| drive_repeat(expr, input, env, emit))
}

fn streamed(
    input: &Value,
    env: &[(String, Value)],
    body: impl FnOnce(&Emit) -> Result<bool, Failure>,
) -> Result<Vec<Value>, Failure> {
    let _ = (input, env);
    let mut out = Vec::new();
    let result = {
        let mut emit = |value: &Value| {
            out.push(value.clone());
            false
        };
        let emit = Emit {
            f: RefCell::new(&mut emit),
        };
        body(&emit)
    };
    finish_emit(result, out)
}

fn finish_emit(result: Result<bool, Failure>, out: Vec<Value>) -> Result<Vec<Value>, Failure> {
    match result {
        Ok(_) => Ok(out),
        Err(mut failure) => {
            let mut partial = out;
            partial.append(&mut failure.partial);
            failure.partial = partial;
            Err(failure)
        }
    }
}

fn drive(
    filter: &Filter,
    input: &Value,
    env: &[(String, Value)],
    emit: &Emit,
) -> Result<bool, Failure> {
    match filter {
        Filter::Pipe(left, right) => drive_pipe(left, right, input, env, emit),
        Filter::Comma(left, right) => drive_comma(left, right, input, env, emit),
        Filter::Limit(count, expr) => drive_limit(count, expr, input, env, emit),
        Filter::First(expr) => drive_first(expr.as_deref(), input, env, emit),
        Filter::Last(expr) => drive_last(expr.as_deref(), input, env, emit),
        Filter::Nth { index, expr } => drive_nth(index, expr.as_deref(), input, env, emit),
        Filter::Range { from, to, step } => {
            drive_range(from.as_deref(), to, step.as_deref(), input, env, emit)
        }
        Filter::Until(cond, next) => drive_until(cond, next, input, env, emit),
        Filter::While(cond, update) => drive_while(cond, update, input, env, emit),
        Filter::Repeat(expr) => drive_repeat(expr, input, env, emit),
        // One value at a time, so `limit` and `first` stop before the next read.
        Filter::Inputs => drive_inputs(emit),
        Filter::Recurse => drive_dots(input, emit),
        Filter::RecurseDown { .. } | Filter::Walk(_) => {
            let mut forward = |value| emit.call(&value);
            super::machine::run(filter, input, env, &mut forward)
        }
        // A call runs on the heap so a function can call itself a million
        // levels deep. Every other filter stays on the direct path. That path
        // is what walks a large array.
        Filter::Define { rest, .. } => drive(rest, input, env, emit),
        Filter::Foreach {
            stream,
            pattern,
            init,
            update,
            extract,
        } => eval::fold_into(
            stream,
            pattern,
            init,
            update,
            extract.as_deref(),
            true,
            input,
            env,
            &mut |value| emit.call(value),
        ),
        Filter::Call { .. } | Filter::Param(_) => {
            let mut forward = |value| emit.call(&value);
            super::machine::run(filter, input, env, &mut forward)
        }
        Filter::Unresolved { name, arity, .. } => {
            Err(fail(format!("{name}/{arity} is not defined")))
        }
        other => relay(eval::produce(other, input, env), emit),
    }
}

pub(super) fn call_values(
    func: &Weak<FnCell>,
    args: &[Filter],
    input: &Value,
    env: &[(String, Value)],
) -> Result<Vec<Value>, Failure> {
    streamed(input, env, |emit| drive_call(func, args, input, env, emit))
}

fn drive_call(
    func: &Weak<FnCell>,
    args: &[Filter],
    input: &Value,
    env: &[(String, Value)],
    emit: &Emit,
) -> Result<bool, Failure> {
    let Some(cell) = func.upgrade() else {
        return Err(fail("function is not defined".to_string()));
    };
    let body = Rc::clone(&cell.body.borrow());
    each_binding(func, args, input, env, &mut |frame| {
        let _guard = func::set_scope(Rc::clone(&frame.scope));
        drive(&body, input, &frame.env, emit)
    })
}

/// Every combination of the `$` arguments, leftmost outermost.
/// A filter argument is not run here. An empty `$` argument runs nothing.
/// A later value is not run once the body asks to stop or fails.
pub(super) fn each_binding(
    func: &Weak<FnCell>,
    args: &[Filter],
    input: &Value,
    env: &[(String, Value)],
    visit: &mut dyn FnMut(&func::Frame) -> Result<bool, Failure>,
) -> Result<bool, Failure> {
    let Some(cell) = func.upgrade() else {
        return Err(fail("function is not defined".to_string()));
    };
    let caller_scope = func::current_scope();
    let link = func::EnvLink::from_slice(env);
    let slots = func::prepare(&cell.params, args, &link, &caller_scope);
    let filters: Vec<(String, Rc<func::Closure>)> = slots.iter().map(func::slot_closure).collect();
    bind_from(
        &slots,
        0,
        &mut Vec::new(),
        &filters,
        input,
        env,
        &caller_scope,
        visit,
    )
}

fn bind_from(
    slots: &[func::Slot],
    index: usize,
    values: &mut Vec<(String, Value)>,
    filters: &[(String, Rc<func::Closure>)],
    input: &Value,
    caller_env: &[(String, Value)],
    caller_scope: &Rc<func::ScopeNode>,
    visit: &mut dyn FnMut(&func::Frame) -> Result<bool, Failure>,
) -> Result<bool, Failure> {
    if index == slots.len() {
        let frame = func::frame(values, filters, caller_env, caller_scope);
        return visit(&frame);
    }
    match &slots[index] {
        func::Slot::Filter { .. } => bind_from(
            slots,
            index + 1,
            values,
            filters,
            input,
            caller_env,
            caller_scope,
            visit,
        ),
        func::Slot::Value { name, expr, .. } => {
            let name = name.clone();
            for_each(expr, input, caller_env, |value| {
                values.push((name.clone(), value));
                let result = bind_from(
                    slots,
                    index + 1,
                    values,
                    filters,
                    input,
                    caller_env,
                    caller_scope,
                    visit,
                );
                values.pop();
                result
            })
        }
    }
}

fn for_each(
    filter: &Filter,
    input: &Value,
    env: &[(String, Value)],
    mut on_value: impl FnMut(Value) -> Result<bool, Failure>,
) -> Result<bool, Failure> {
    let failure: RefCell<Option<Failure>> = RefCell::new(None);
    let mut on_one = |value: &Value| match on_value(value.clone()) {
        Ok(stop) => stop,
        Err(err) => {
            *failure.borrow_mut() = Some(err);
            true
        }
    };
    let emit = Emit {
        f: RefCell::new(&mut on_one),
    };
    let result = drive(filter, input, env, &emit);
    if let Some(err) = failure.into_inner() {
        return Err(err);
    }
    result
}

fn relay(result: Result<Vec<Value>, Failure>, emit: &Emit) -> Result<bool, Failure> {
    let (values, err) = match result {
        Ok(values) => (values, None),
        Err(mut failure) => (std::mem::take(&mut failure.partial), Some(failure)),
    };
    for value in values {
        if emit.call(&value) {
            return Ok(true);
        }
    }
    match err {
        Some(failure) => Err(failure),
        None => Ok(false),
    }
}

/// A right-hand error stops the left. Values already produced are kept.
fn drive_pipe(
    left: &Filter,
    right: &Filter,
    input: &Value,
    env: &[(String, Value)],
    emit: &Emit,
) -> Result<bool, Failure> {
    let failure: RefCell<Option<Failure>> = RefCell::new(None);
    let stopped = Cell::new(false);
    let mut on_left = |value: &Value| {
        if stopped.get() {
            return true;
        }
        match drive(right, value, env, emit) {
            Ok(stop) => {
                if stop {
                    stopped.set(true);
                }
                stop
            }
            Err(err) => {
                *failure.borrow_mut() = Some(err);
                true
            }
        }
    };
    let on_left = Emit {
        f: RefCell::new(&mut on_left),
    };
    let left_result = drive(left, input, env, &on_left);
    if let Some(err) = failure.into_inner() {
        return Err(err);
    }
    let left_stopped = left_result?;
    Ok(stopped.get() || left_stopped)
}

fn drive_comma(
    left: &Filter,
    right: &Filter,
    input: &Value,
    env: &[(String, Value)],
    emit: &Emit,
) -> Result<bool, Failure> {
    if drive(left, input, env, emit)? {
        return Ok(true);
    }
    drive(right, input, env, emit)
}

/// How many values `limit` keeps. `All` is a negative count, `nan`, or an
/// infinite count: the whole stream, including a later error. `None` is zero
/// and does not run the stream. `Count` is `ceil` of a positive finite number.
/// Anything else is subtracted after the first value so jq's type error wins
/// over a value, and a failing stream wins over that subtraction.
pub(super) enum LimitHow {
    All,
    None,
    Count(usize),
    Subtract,
}

pub(super) fn limit_how(count: &Value) -> LimitHow {
    let zero = Value::Number("0".to_string());
    if cmp_value(count, &zero) < 0 {
        return LimitHow::All;
    }
    if cmp_value(count, &zero) <= 0 {
        return LimitHow::None;
    }
    match count {
        Value::Number(text) => {
            let n = number::f64_of(text);
            if !n.is_finite() {
                return LimitHow::All;
            }
            let slots = n.ceil();
            if slots <= 0.0 {
                LimitHow::None
            } else if slots >= usize::MAX as f64 {
                LimitHow::All
            } else {
                LimitHow::Count(slots as usize)
            }
        }
        _ => LimitHow::Subtract,
    }
}

fn drive_limit(
    count: &Filter,
    expr: &Filter,
    input: &Value,
    env: &[(String, Value)],
    emit: &Emit,
) -> Result<bool, Failure> {
    let failure: RefCell<Option<Failure>> = RefCell::new(None);
    let stopped = Cell::new(false);
    let mut on_count = |count: &Value| {
        if stopped.get() {
            return true;
        }
        match take_count(count, expr, input, env, emit) {
            Ok(stop) => {
                if stop {
                    stopped.set(true);
                }
                stop
            }
            Err(err) => {
                *failure.borrow_mut() = Some(err);
                true
            }
        }
    };
    let on_count = Emit {
        f: RefCell::new(&mut on_count),
    };
    let result = drive(count, input, env, &on_count);
    if let Some(err) = failure.into_inner() {
        return Err(err);
    }
    let count_stopped = result?;
    Ok(stopped.get() || count_stopped)
}

fn take_count(
    count: &Value,
    expr: &Filter,
    input: &Value,
    env: &[(String, Value)],
    emit: &Emit,
) -> Result<bool, Failure> {
    match limit_how(count) {
        LimitHow::None => Ok(false),
        LimitHow::All => drive(expr, input, env, emit),
        LimitHow::Count(need) => {
            let seen = Cell::new(0usize);
            let outer = Cell::new(false);
            let mut on_value = |value: &Value| {
                let n = seen.get() + 1;
                seen.set(n);
                if emit.call(value) {
                    outer.set(true);
                    return true;
                }
                n >= need
            };
            let on_value = Emit {
                f: RefCell::new(&mut on_value),
            };
            drive(expr, input, env, &on_value)?;
            Ok(outer.get())
        }
        LimitHow::Subtract => {
            let left = RefCell::new(count.clone());
            let zero = Value::Number("0".to_string());
            let one = Value::Number("1".to_string());
            let outer = Cell::new(false);
            let failure: RefCell<Option<Failure>> = RefCell::new(None);
            let mut on_value = |value: &Value| {
                let next = match sub_values(&left.borrow(), &one) {
                    Ok(next) => next,
                    Err(message) => {
                        *failure.borrow_mut() = Some(fail(message));
                        return true;
                    }
                };
                *left.borrow_mut() = next;
                if emit.call(value) {
                    outer.set(true);
                    return true;
                }
                cmp_value(&left.borrow(), &zero) <= 0
            };
            let on_value = Emit {
                f: RefCell::new(&mut on_value),
            };
            let result = drive(expr, input, env, &on_value);
            if let Some(err) = failure.into_inner() {
                return Err(err);
            }
            result?;
            Ok(outer.get())
        }
    }
}

fn drive_first(
    expr: Option<&Filter>,
    input: &Value,
    env: &[(String, Value)],
    emit: &Emit,
) -> Result<bool, Failure> {
    let Some(expr) = expr else {
        return relay(index_by(input, &Value::Number("0".to_string())), emit);
    };
    let outer = Cell::new(false);
    let mut on_value = |value: &Value| {
        outer.set(emit.call(value));
        true
    };
    let on_value = Emit {
        f: RefCell::new(&mut on_value),
    };
    drive(expr, input, env, &on_value)?;
    Ok(outer.get())
}

fn drive_last(
    expr: Option<&Filter>,
    input: &Value,
    env: &[(String, Value)],
    emit: &Emit,
) -> Result<bool, Failure> {
    let Some(expr) = expr else {
        return relay(index_by(input, &Value::Number("-1".to_string())), emit);
    };
    let last: RefCell<Option<Value>> = RefCell::new(None);
    let mut on_value = |value: &Value| {
        *last.borrow_mut() = Some(value.clone());
        false
    };
    let on_value = Emit {
        f: RefCell::new(&mut on_value),
    };
    // A failure drops every value. `last` only emits after the stream ends.
    drive(expr, input, env, &on_value)?;
    match last.into_inner() {
        Some(value) => Ok(emit.call(&value)),
        None => Ok(emit.call(&Value::Null)),
    }
}

/// Which output of `nth(n; expr)` is kept. `Take(k)` reads `k` outputs and
/// emits the last of them. Fewer outputs, and no error, emit nothing.
pub(super) enum NthSpan {
    Negative,
    Add(String),
    Never,
    Take(usize),
}

pub(super) fn nth_span(index: &Value) -> NthSpan {
    let zero = Value::Number("0".to_string());
    if cmp_value(index, &zero) < 0 {
        return NthSpan::Negative;
    }
    let summed = match add_values(index, &Value::Number("1".to_string())) {
        Ok(value) => value,
        Err(message) => return NthSpan::Add(message),
    };
    let Value::Number(text) = summed else {
        return NthSpan::Add("number required".to_string());
    };
    let n = number::f64_of(&text);
    if !n.is_finite() {
        return NthSpan::Never;
    }
    if n <= 0.0 {
        return NthSpan::Take(0);
    }
    let slots = n.ceil();
    if slots >= usize::MAX as f64 {
        NthSpan::Never
    } else {
        NthSpan::Take(slots as usize)
    }
}

fn drive_nth(
    index: &Filter,
    expr: Option<&Filter>,
    input: &Value,
    env: &[(String, Value)],
    emit: &Emit,
) -> Result<bool, Failure> {
    let Some(expr) = expr else {
        return relay(
            eval::eval_at(&Filter::Identity, index, false, input, env),
            emit,
        );
    };
    let failure: RefCell<Option<Failure>> = RefCell::new(None);
    let stopped = Cell::new(false);
    let mut on_index = |index: &Value| {
        if stopped.get() {
            return true;
        }
        match take_nth(index, expr, input, env, emit) {
            Ok(stop) => {
                if stop {
                    stopped.set(true);
                }
                stop
            }
            Err(err) => {
                *failure.borrow_mut() = Some(err);
                true
            }
        }
    };
    let on_index = Emit {
        f: RefCell::new(&mut on_index),
    };
    let result = drive(index, input, env, &on_index);
    if let Some(err) = failure.into_inner() {
        return Err(err);
    }
    let index_stopped = result?;
    Ok(stopped.get() || index_stopped)
}

fn take_nth(
    index: &Value,
    expr: &Filter,
    input: &Value,
    env: &[(String, Value)],
    emit: &Emit,
) -> Result<bool, Failure> {
    match nth_span(index) {
        NthSpan::Negative => Err(fail("nth doesn't support negative indices".to_string())),
        NthSpan::Add(message) => Err(fail(message)),
        NthSpan::Take(0) => Ok(false),
        NthSpan::Never => {
            let mut ignore = |_value: &Value| false;
            let ignore = Emit {
                f: RefCell::new(&mut ignore),
            };
            drive(expr, input, env, &ignore)?;
            Ok(false)
        }
        NthSpan::Take(need) => {
            let seen = Cell::new(0usize);
            let outer = Cell::new(false);
            let mut on_value = |value: &Value| {
                let n = seen.get() + 1;
                seen.set(n);
                if n < need {
                    return false;
                }
                outer.set(emit.call(value));
                true
            };
            let on_value = Emit {
                f: RefCell::new(&mut on_value),
            };
            drive(expr, input, env, &on_value)?;
            Ok(outer.get())
        }
    }
}

fn drive_range(
    from: Option<&Filter>,
    to: &Filter,
    step: Option<&Filter>,
    input: &Value,
    env: &[(String, Value)],
    emit: &Emit,
) -> Result<bool, Failure> {
    // The left argument is the outer loop. An omitted start is 0 and an
    // omitted step is 1. One and two arguments treat a `nan` end as no end.
    let implicit = step.is_none();
    let failure: RefCell<Option<Failure>> = RefCell::new(None);
    let stopped = Cell::new(false);
    let zero = Value::Number("0".to_string());
    let one = Value::Number("1".to_string());
    let mut on_from = |from_value: &Value| {
        if stopped.get() {
            return true;
        }
        let mut on_to = |to_value: &Value| {
            if stopped.get() {
                return true;
            }
            let mut on_step = |step_value: &Value| {
                if stopped.get() {
                    return true;
                }
                match gen_range(from_value, to_value, step_value, implicit, emit) {
                    Ok(stop) => {
                        if stop {
                            stopped.set(true);
                        }
                        stop
                    }
                    Err(err) => {
                        *failure.borrow_mut() = Some(err);
                        true
                    }
                }
            };
            let on_step = Emit {
                f: RefCell::new(&mut on_step),
            };
            match drive_fixed(step, &one, input, env, &on_step) {
                Ok(stop) => {
                    if stop {
                        stopped.set(true);
                    }
                    stop
                }
                Err(err) => {
                    *failure.borrow_mut() = Some(err);
                    true
                }
            }
        };
        let on_to = Emit {
            f: RefCell::new(&mut on_to),
        };
        match drive(to, input, env, &on_to) {
            Ok(stop) => {
                if stop {
                    stopped.set(true);
                }
                stop
            }
            Err(err) => {
                *failure.borrow_mut() = Some(err);
                true
            }
        }
    };
    let on_from = Emit {
        f: RefCell::new(&mut on_from),
    };
    let result = drive_fixed(from, &zero, input, env, &on_from);
    if let Some(err) = failure.into_inner() {
        return Err(err);
    }
    let from_stopped = result?;
    Ok(stopped.get() || from_stopped)
}

fn drive_fixed(
    filter: Option<&Filter>,
    fixed: &Value,
    input: &Value,
    env: &[(String, Value)],
    emit: &Emit,
) -> Result<bool, Failure> {
    match filter {
        Some(filter) => drive(filter, input, env, emit),
        None => Ok(emit.call(fixed)),
    }
}

fn gen_range(
    from: &Value,
    to: &Value,
    step: &Value,
    implicit_unit: bool,
    emit: &Emit,
) -> Result<bool, Failure> {
    if !matches!(from, Value::Number(_)) || !matches!(to, Value::Number(_)) {
        return Err(fail("Range bounds must be numeric".to_string()));
    }
    let mut cur = from.clone();
    loop {
        if !keeps_going(&cur, to, step, implicit_unit) {
            return Ok(false);
        }
        if emit.call(&cur) {
            return Ok(true);
        }
        cur = add_values(&cur, step).map_err(fail)?;
    }
}

fn keeps_going(from: &Value, to: &Value, step: &Value, implicit_unit: bool) -> bool {
    let zero = Value::Number("0".to_string());
    if cmp_value(step, &zero) > 0 {
        // `range(from; nan)` and `range(nan)` do not stop. `range(from; nan; 1)` does.
        cmp_value(from, to) < 0 || (implicit_unit && is_nan(to))
    } else if cmp_value(step, &zero) < 0 {
        cmp_value(from, to) > 0
    } else {
        false
    }
}

fn is_nan(value: &Value) -> bool {
    match value {
        Value::Number(text) => matches!(number::real(text), number::Real::Nan),
        _ => false,
    }
}

fn drive_until(
    cond: &Filter,
    next: &Filter,
    input: &Value,
    env: &[(String, Value)],
    emit: &Emit,
) -> Result<bool, Failure> {
    // Each cond value is its own branch. A false one runs `next` before the
    // next cond value, which is why `until(.>3, error("z"); .+1)` reaches 4.
    let failure: RefCell<Option<Failure>> = RefCell::new(None);
    let stopped = Cell::new(false);
    let mut on_flag = |flag: &Value| {
        if stopped.get() {
            return true;
        }
        if eval::truthy(flag) {
            if emit.call(input) {
                stopped.set(true);
            }
            return stopped.get();
        }
        let mut on_next = |value: &Value| {
            if stopped.get() {
                return true;
            }
            match drive_until(cond, next, value, env, emit) {
                Ok(stop) => {
                    if stop {
                        stopped.set(true);
                    }
                    stop
                }
                Err(err) => {
                    *failure.borrow_mut() = Some(err);
                    true
                }
            }
        };
        let on_next = Emit {
            f: RefCell::new(&mut on_next),
        };
        match drive(next, input, env, &on_next) {
            Ok(stop) => {
                if stop {
                    stopped.set(true);
                }
                stop
            }
            Err(err) => {
                *failure.borrow_mut() = Some(err);
                true
            }
        }
    };
    let on_flag = Emit {
        f: RefCell::new(&mut on_flag),
    };
    let result = drive(cond, input, env, &on_flag);
    if let Some(err) = failure.into_inner() {
        return Err(err);
    }
    let cond_stopped = result?;
    Ok(stopped.get() || cond_stopped)
}

fn drive_while(
    cond: &Filter,
    update: &Filter,
    input: &Value,
    env: &[(String, Value)],
    emit: &Emit,
) -> Result<bool, Failure> {
    // Emit the input once for each truthy cond value, then walk every update
    // output depth-first. A false cond value emits nothing.
    let failure: RefCell<Option<Failure>> = RefCell::new(None);
    let stopped = Cell::new(false);
    let mut on_flag = |flag: &Value| {
        if stopped.get() {
            return true;
        }
        if !eval::truthy(flag) {
            return false;
        }
        if emit.call(input) {
            stopped.set(true);
            return true;
        }
        let mut on_update = |value: &Value| {
            if stopped.get() {
                return true;
            }
            match drive_while(cond, update, value, env, emit) {
                Ok(stop) => {
                    if stop {
                        stopped.set(true);
                    }
                    stop
                }
                Err(err) => {
                    *failure.borrow_mut() = Some(err);
                    true
                }
            }
        };
        let on_update = Emit {
            f: RefCell::new(&mut on_update),
        };
        match drive(update, input, env, &on_update) {
            Ok(stop) => {
                if stop {
                    stopped.set(true);
                }
                stop
            }
            Err(err) => {
                *failure.borrow_mut() = Some(err);
                true
            }
        }
    };
    let on_flag = Emit {
        f: RefCell::new(&mut on_flag),
    };
    let result = drive(cond, input, env, &on_flag);
    if let Some(err) = failure.into_inner() {
        return Err(err);
    }
    let cond_stopped = result?;
    Ok(stopped.get() || cond_stopped)
}

/// `..` and `recurse`. Parent first, then each element or object value.
fn drive_dots(input: &Value, emit: &Emit) -> Result<bool, Failure> {
    let mut stack = vec![input.clone()];
    while let Some(value) = stack.pop() {
        if let Ok(mut children) = super::eval::iterate(&value) {
            children.reverse();
            stack.extend(children);
        }
        if emit.call(&value) {
            return Ok(true);
        }
    }
    Ok(false)
}

pub(super) fn recurse_values(
    step: &Filter,
    cond: Option<&Filter>,
    input: &Value,
    env: &[(String, Value)],
) -> Result<Vec<Value>, Failure> {
    streamed(input, env, |emit| {
        super::machine::run(
            &Filter::RecurseDown {
                step: Box::new(step.clone()),
                cond: cond.map(|cond| Box::new(cond.clone())),
            },
            input,
            env,
            &mut |value| emit.call(&value),
        )
    })
}

pub(super) fn walk_values(
    update: &Filter,
    input: &Value,
    env: &[(String, Value)],
) -> Result<Vec<Value>, Failure> {
    streamed(input, env, |emit| {
        super::machine::run(
            &Filter::Walk(Box::new(update.clone())),
            input,
            env,
            &mut |value| emit.call(&value),
        )
    })
}

/// The `index`th value of `filter`, or nothing when the stream ends first.
/// Later values are not run. An earlier error is returned.
pub(super) fn nth_value(
    filter: &Filter,
    input: &Value,
    env: &[(String, Value)],
    index: usize,
) -> Result<Option<Value>, Failure> {
    let mut seen = 0usize;
    let mut found = None;
    let mut on_value = |value: &Value| {
        if seen == index {
            found = Some(value.clone());
            return true;
        }
        seen += 1;
        false
    };
    let emit = Emit {
        f: RefCell::new(&mut on_value),
    };
    match drive(filter, input, env, &emit) {
        Ok(_) => Ok(found),
        Err(_) if found.is_some() => Ok(found),
        Err(err) => Err(err),
    }
}

/// Whether `cond` has an `index`th truthy output. The stream stops there, so
/// a later error is left for the next index.
pub(super) fn nth_truthy(
    filter: &Filter,
    input: &Value,
    env: &[(String, Value)],
    index: usize,
) -> Result<bool, Failure> {
    let mut truths = 0usize;
    let mut hit = false;
    let mut on_value = |value: &Value| {
        if !super::eval::truthy(value) {
            return false;
        }
        if truths == index {
            hit = true;
            return true;
        }
        truths += 1;
        false
    };
    let emit = Emit {
        f: RefCell::new(&mut on_value),
    };
    match drive(filter, input, env, &emit) {
        Ok(_) => Ok(hit),
        Err(err) if hit => {
            let _ = err;
            Ok(true)
        }
        Err(err) => Err(err),
    }
}

fn drive_inputs(emit: &Emit) -> Result<bool, Failure> {
    loop {
        match crate::feed::read_one() {
            crate::feed::Read::Value(value) => {
                if emit.call(&value) {
                    return Ok(true);
                }
            }
            // The end of the stream is not a result. A bad value still fails.
            crate::feed::Read::Break => return Ok(false),
            crate::feed::Read::Parse(message) | crate::feed::Read::Fatal(message) => {
                return Err(fail(message));
            }
        }
    }
}

fn drive_repeat(
    expr: &Filter,
    input: &Value,
    env: &[(String, Value)],
    emit: &Emit,
) -> Result<bool, Failure> {
    loop {
        if drive(expr, input, env, emit)? {
            return Ok(true);
        }
    }
}
