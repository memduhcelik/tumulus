//! Path updates. A path is collected from the input, then each write is applied
//! in that order. Array indexes keep their original slots until the update
//! finishes, so a later index is not shifted by an earlier deletion.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;

use crate::number::array_index;
use crate::value::Value;

use super::stream::{self, LimitHow, NthSpan};
use super::{
    add_values, bare_fail, canonical_number, child_scope, clipped, div_values, eval_in, fail,
    finish, fresh, mul_values, parts, put, rem_values, require_slice_index, shown,
    slice as slice_value, slice_range, sub_values, truthy, walk_slice_pairs, with_partial,
    ArrayIndex, Failure, Filter, ObjField, Op, PatField, Pattern, Stop, UpdateOp,
};

enum Found {
    Path(Vec<Step>),
    /// A value that was computed instead of reached through the input.
    Broken(Value),
    /// `.[nan]` is a path jq will print, and setting it has its own message.
    NanIndex,
}

#[derive(Clone)]
enum Step {
    Key(String),
    /// `at` is the slot updates write. `text` is what `path` prints, so
    /// `1.5`, `1.0`, and `-0` keep the spelling jq prints.
    Index {
        at: i64,
        text: String,
    },
    /// A number that does not fit in `at`. `path` prints `text`. Updates
    /// still see a broken path, and this step is not written.
    Huge(String),
    /// A `getpath` component printed as itself. An array or a slice object.
    Raw(Value),
    /// `start` and `end` are the values the bounds produced. Null is an
    /// omitted side or a null bound. A number is an index. Anything else is
    /// kept only while the value being sliced is null.
    Slice {
        start: Value,
        end: Value,
    },
}

enum Edit {
    Pure(Value),
    Obj(Vec<(String, Edit)>),
    Arr(Arr),
}

struct Arr {
    slots: Vec<Elem>,
    tail: Vec<Edit>,
}

struct Elem {
    /// Inserted by an empty slice just before this original slot.
    before: Vec<Edit>,
    body: Body,
}

enum Body {
    Live(Edit),
    Gone,
}

enum Write {
    Put(Value),
    Remove,
}

pub fn eval_update(
    op: UpdateOp,
    path: &Filter,
    expr: &Filter,
    input: &Value,
    env: &[(String, Value)],
) -> Result<Vec<Value>, Failure> {
    if op == UpdateOp::Modify {
        return update_modify(path, expr, input, env);
    }
    // One right-hand value is applied to every place, then the next value
    // starts again from the input. An empty stream yields nothing and does
    // not look at the path. A failure while producing a value, before any
    // value has been written, is reported before a bad path is.
    update_values(op, path, expr, input, env)
}

/// `=` and the arithmetic updates. Each value is written from the original
/// input. A write that cannot be done does not pull another value.
fn update_values(
    op: UpdateOp,
    path: &Filter,
    expr: &Filter,
    input: &Value,
    env: &[(String, Value)],
) -> Result<Vec<Value>, Failure> {
    let mut outs = Vec::new();
    let write_err: RefCell<Option<Failure>> = RefCell::new(None);
    let fed = stream::feed(expr, input, env, &mut |value| match apply_rhs(
        op, path, input, env, value,
    ) {
        Ok(produced) => {
            outs.push(produced);
            false
        }
        Err(failure) => {
            *write_err.borrow_mut() = Some(failure);
            true
        }
    });
    if let Some(failure) = write_err.into_inner() {
        return Err(with_partial(outs, failure));
    }
    match fed {
        Ok(_) => Ok(outs),
        Err(mut failure) => {
            // Values the stream produced beside the error, and did not hand
            // over, are still right-hand values.
            let extra = std::mem::take(&mut failure.partial);
            for value in extra {
                match apply_rhs(op, path, input, env, &value) {
                    Ok(produced) => outs.push(produced),
                    Err(err) => return Err(with_partial(outs, err)),
                }
            }
            Err(with_partial(outs, failure))
        }
    }
}

fn apply_rhs(
    op: UpdateOp,
    path: &Filter,
    input: &Value,
    env: &[(String, Value)],
    rhs: &Value,
) -> Result<Value, Failure> {
    let mut edit = Edit::Pure(input.clone());
    let mut saw = false;
    let result = drive_paths(path, input, env, &mut |out| {
        saw = true;
        apply_write(op, &mut edit, input, out, rhs)
    });
    match result {
        Err(WalkErr::Path(err) | WalkErr::Write(err)) => Err(err),
        Err(WalkErr::Enough) | Ok(()) if !saw => Ok(input.clone()),
        Err(WalkErr::Enough) | Ok(()) => Ok(freeze(&edit)),
    }
}

fn apply_write(
    op: UpdateOp,
    edit: &mut Edit,
    root: &Value,
    out: POut,
    rhs: &Value,
) -> Result<(), WalkErr> {
    let steps = steps_of(root, out)?;
    let value = match op {
        UpdateOp::Assign => rhs.clone(),
        UpdateOp::Add => combine(edit, &steps, rhs, add_values)?,
        UpdateOp::Sub => combine(edit, &steps, rhs, sub_values)?,
        UpdateOp::Mul => combine(edit, &steps, rhs, mul_values)?,
        UpdateOp::Div => combine(edit, &steps, rhs, div_values)?,
        UpdateOp::Rem => combine(edit, &steps, rhs, rem_values)?,
        UpdateOp::Alt => {
            let left = read_edit(edit, &steps).map_err(|message| WalkErr::Write(fail(message)))?;
            if truthy(&left) {
                left
            } else {
                rhs.clone()
            }
        }
        UpdateOp::Modify => unreachable!("modify"),
    };
    write_path(edit, &steps, &Write::Put(value))
        .map_err(|message| WalkErr::Write(fail(message)))?;
    Ok(())
}

fn combine(
    edit: &Edit,
    steps: &[Step],
    rhs: &Value,
    join: fn(&Value, &Value) -> Result<Value, String>,
) -> Result<Value, WalkErr> {
    let left = read_edit(edit, steps).map_err(|message| WalkErr::Write(fail(message)))?;
    join(&left, rhs).map_err(|message| WalkErr::Write(fail(message)))
}

/// `|=` and `del`. The filter runs at one place before the next bound.
/// Deleting from a string does not stop the walk: a later bound error is
/// the one reported, and with no later error the string error is.
fn update_modify(
    path: &Filter,
    expr: &Filter,
    input: &Value,
    env: &[(String, Value)],
) -> Result<Vec<Value>, Failure> {
    let mut edit = Edit::Pure(input.clone());
    let mut saw = false;
    let mut string_del: Option<Failure> = None;
    let result = drive_paths(path, input, env, &mut |out| {
        saw = true;
        apply_modify(&mut edit, input, out, expr, env, &mut string_del)
    });
    match result {
        Err(WalkErr::Path(err) | WalkErr::Write(err)) => Err(err),
        Err(WalkErr::Enough) | Ok(()) => {
            if let Some(err) = string_del {
                return Err(err);
            }
            if !saw {
                Ok(vec![input.clone()])
            } else {
                Ok(vec![freeze(&edit)])
            }
        }
    }
}

fn apply_modify(
    edit: &mut Edit,
    root: &Value,
    out: POut,
    expr: &Filter,
    env: &[(String, Value)],
    string_del: &mut Option<Failure>,
) -> Result<(), WalkErr> {
    let steps = steps_of(root, out)?;
    let current = read_edit(edit, &steps).map_err(|message| WalkErr::Write(fail(message)))?;
    let write = match eval_first(expr, &current, env) {
        Ok(None) => Write::Remove,
        Ok(Some(value)) => Write::Put(value),
        Err(err) => return Err(WalkErr::Write(err)),
    };
    if let Err(message) = write_path(edit, &steps, &write) {
        if message == "Cannot delete fields from string" {
            *string_del = Some(fail(message));
            return Ok(());
        }
        return Err(WalkErr::Write(fail(message)));
    }
    Ok(())
}

fn steps_of(root: &Value, out: POut) -> Result<Vec<Step>, WalkErr> {
    match classify_one(root, out) {
        Found::Path(steps) => Ok(steps),
        Found::Broken(value) => Err(WalkErr::Path(fail(format!(
            "Invalid path expression with result {}",
            clipped(&value, 29)
        )))),
        Found::NanIndex => Err(WalkErr::Path(fail(
            "Cannot set array element at NaN index".to_string(),
        ))),
    }
}

/// A path failure can be swallowed by `(expr)?`. A write failure cannot,
/// and it stops the walk before the next slice bound. `Enough` stops the
/// walk after `first` or `limit` has the values it needs.
enum WalkErr {
    Path(Failure),
    Write(Failure),
    Enough,
}

fn drive_paths(
    path: &Filter,
    input: &Value,
    env: &[(String, Value)],
    emit: &mut dyn FnMut(POut) -> Result<(), WalkErr>,
) -> Result<(), WalkErr> {
    let next = Cell::new(2);
    let line = Cell::new(1);
    let ctx = Ctx {
        root: input,
        next: &next,
        env,
        pbinds: &[],
    };
    send(path, input, &[], &line, 1, &ctx, emit)
}

/// Hand each place to `emit` as soon as it is known. A slice bound is not
/// pulled until `emit` has returned for the pair before it.
fn send(
    filter: &Filter,
    input: &Value,
    path: &[Step],
    line: &Cell<u64>,
    input_gen: u64,
    ctx: &Ctx,
    emit: &mut dyn FnMut(POut) -> Result<(), WalkErr>,
) -> Result<(), WalkErr> {
    match filter {
        Filter::Slice {
            base,
            start,
            end,
            optional,
        } => send_slice(
            base.as_deref(),
            start.as_deref(),
            end.as_deref(),
            *optional,
            input,
            path,
            line,
            input_gen,
            ctx,
            emit,
        ),
        Filter::At {
            base,
            key,
            optional,
        } => send_at(
            base, key, *optional, input, path, line, input_gen, ctx, emit,
        ),
        Filter::Comma(left, right) => {
            // Each side starts from the lineage the comma was reached at.
            // An index on the left must not make the right look computed.
            let saved = line.get();
            let left_cell = Cell::new(saved);
            send(left, input, path, &left_cell, input_gen, ctx, emit)?;
            let right_cell = Cell::new(saved);
            send(right, input, path, &right_cell, input_gen, ctx, emit)
        }
        Filter::Pipe(left, right) if !matches!(left.as_ref(), Filter::MatchImpl { .. }) => {
            send_pipe(left, right, input, path, line, input_gen, ctx, emit)
        }
        Filter::Alt(left, right) => send_alt(left, right, input, path, line, input_gen, ctx, emit),
        Filter::If { cond, yes, no } => {
            send_if(cond, yes, no, input, path, line, input_gen, ctx, emit)
        }
        Filter::Bind {
            expr,
            pattern,
            alts,
            body,
        } if has_slice(body) => send_bind(
            expr, pattern, alts, body, input, path, line, input_gen, ctx, emit,
        ),
        Filter::Optional(inner) => send_optional(inner, input, path, line, input_gen, ctx, emit),
        Filter::Call { func, args } => {
            send_call(func, args, input, path, line, input_gen, ctx, emit)
        }
        Filter::Param(name) => send_param(name, input, path, line, input_gen, ctx, emit),
        Filter::Define { rest, .. } => send(rest, input, path, line, input_gen, ctx, emit),
        Filter::Label { slot, body } => {
            send_label(*slot, body, input, path, line, input_gen, ctx, emit)
        }
        Filter::First(Some(inner)) => send_take(inner, 1, input, path, line, input_gen, ctx, emit),
        Filter::Limit(count, expr) => {
            send_limit(count, expr, input, path, line, input_gen, ctx, emit)
        }
        _ => emit_run(filter, input, path, line, input_gen, ctx, emit),
    }
}

fn send_slice(
    base: Option<&Filter>,
    start: Option<&Filter>,
    end: Option<&Filter>,
    optional: bool,
    input: &Value,
    path: &[Step],
    line: &Cell<u64>,
    input_gen: u64,
    ctx: &Ctx,
    emit: &mut dyn FnMut(POut) -> Result<(), WalkErr>,
) -> Result<(), WalkErr> {
    let identity = Filter::Identity;
    let base = base.unwrap_or(&identity);
    let saved = line.get();
    let slot: RefCell<Option<WalkErr>> = RefCell::new(None);
    let walked = walk_slice_pairs(start, end, input, ctx.env, |start_value, end_value| {
        line.set(saved);
        let sent = send(base, input, path, line, input_gen, ctx, &mut |base_out| {
            finish_slice(base_out, start_value, end_value, optional, line, ctx, emit)
        });
        keep_walk(sent, &slot)
    });
    if let Some(err) = slot.into_inner() {
        return Err(err);
    }
    match walked {
        Ok(()) => Ok(()),
        Err(failure) => Err(WalkErr::Path(failure)),
    }
}

fn finish_slice(
    base_out: POut,
    start_value: &Value,
    end_value: &Value,
    optional: bool,
    line: &Cell<u64>,
    ctx: &Ctx,
    emit: &mut dyn FnMut(POut) -> Result<(), WalkErr>,
) -> Result<(), WalkErr> {
    if !base_out.carrier {
        let elem = shown_slice(start_value, end_value);
        return Err(WalkErr::Path(fail(near_access(&elem, &base_out.value))));
    }
    match &base_out.value {
        Value::Array(_) | Value::String(_) => {
            if require_slice_index(start_value).is_err() || require_slice_index(end_value).is_err()
            {
                if optional {
                    return Ok(());
                }
                require_slice_index(start_value).map_err(WalkErr::Path)?;
                require_slice_index(end_value).map_err(WalkErr::Path)?;
            }
        }
        Value::Null => {}
        other => {
            if optional {
                return Ok(());
            }
            return Err(WalkErr::Path(fail(format!(
                "Cannot index {} with object",
                other.type_name()
            ))));
        }
    }
    let step = Step::Slice {
        start: start_value.clone(),
        end: end_value.clone(),
    };
    let child = child_at(&base_out.value, &step).map_err(|message| WalkErr::Path(fail(message)))?;
    let mut full = base_out.path;
    full.push(step);
    emit(taken(child, full, line, ctx.next))
}

fn send_at(
    base: &Filter,
    key: &Filter,
    optional: bool,
    input: &Value,
    path: &[Step],
    line: &Cell<u64>,
    input_gen: u64,
    ctx: &Ctx,
    emit: &mut dyn FnMut(POut) -> Result<(), WalkErr>,
) -> Result<(), WalkErr> {
    let saved = line.get();
    let slot: RefCell<Option<WalkErr>> = RefCell::new(None);
    let result = stream::for_each_value(key, input, ctx.env, |key_value| {
        line.set(saved);
        let sent = send(
            base,
            input,
            path,
            line,
            input_gen,
            ctx,
            &mut |base_out| match index_out(&base_out, key_value, ctx) {
                Ok(hit) => emit(hit),
                Err(message) if optional && skips_failed_index(&message) => {
                    let _ = message;
                    Ok(())
                }
                Err(message) => Err(WalkErr::Path(fail(message))),
            },
        );
        // `true` stops the key stream. The walk error is kept aside so a
        // write failure is not turned into a plain stream error.
        match sent {
            Ok(()) => Ok(false),
            Err(err) => {
                *slot.borrow_mut() = Some(err);
                Ok(true)
            }
        }
    });
    if let Some(err) = slot.into_inner() {
        return Err(err);
    }
    match result {
        Ok(_) => Ok(()),
        Err(failure) => Err(WalkErr::Path(failure)),
    }
}

fn send_alt(
    left: &Filter,
    right: &Filter,
    input: &Value,
    path: &[Step],
    line: &Cell<u64>,
    input_gen: u64,
    ctx: &Ctx,
    emit: &mut dyn FnMut(POut) -> Result<(), WalkErr>,
) -> Result<(), WalkErr> {
    let saved_line = line.get();
    let saved_path = path.to_vec();
    let saw = Cell::new(false);
    let left_result = send(left, input, path, line, input_gen, ctx, &mut |out| {
        if !truthy(&out.value) {
            return Ok(());
        }
        saw.set(true);
        emit(out)
    });
    match left_result {
        Err(err) => Err(err),
        Ok(()) if saw.get() => Ok(()),
        Ok(()) => {
            line.set(saved_line);
            send(right, input, &saved_path, line, input_gen, ctx, emit)
        }
    }
}

fn send_if(
    cond: &Filter,
    yes: &Filter,
    no: &Filter,
    input: &Value,
    path: &[Step],
    line: &Cell<u64>,
    input_gen: u64,
    ctx: &Ctx,
    emit: &mut dyn FnMut(POut) -> Result<(), WalkErr>,
) -> Result<(), WalkErr> {
    let saved = line.get();
    let (conds, err) = parts(eval_in(cond, input, ctx.env));
    line.set(saved);
    for cond_value in &conds {
        line.set(saved);
        let branch = if truthy(cond_value) { yes } else { no };
        send(branch, input, path, line, input_gen, ctx, emit)?;
    }
    match err {
        Some(failure) => Err(WalkErr::Path(failure)),
        None => Ok(()),
    }
}

/// True when a later slice bound can sit inside `filter`, so the update has
/// to write each place before the next bound runs.
fn has_slice(filter: &Filter) -> bool {
    match filter {
        Filter::Slice { .. } | Filter::Call { .. } | Filter::Param(_) => true,
        Filter::At { base, .. } => has_slice(base),
        Filter::Comma(left, right)
        | Filter::Pipe(left, right)
        | Filter::Alt(left, right)
        | Filter::And(left, right)
        | Filter::Or(left, right) => has_slice(left) || has_slice(right),
        Filter::Optional(inner) | Filter::Define { rest: inner, .. } => has_slice(inner),
        Filter::If { yes, no, .. } => has_slice(yes) || has_slice(no),
        Filter::Bind { body, .. } => has_slice(body),
        _ => false,
    }
}

fn send_bind(
    expr: &Filter,
    pattern: &Pattern,
    alts: &[Pattern],
    body: &Filter,
    input: &Value,
    path: &[Step],
    line: &Cell<u64>,
    input_gen: u64,
    ctx: &Ctx,
    emit: &mut dyn FnMut(POut) -> Result<(), WalkErr>,
) -> Result<(), WalkErr> {
    let mut pats = Vec::with_capacity(1 + alts.len());
    pats.push(pattern);
    pats.extend(alts);
    // Every `?//` pattern's names are in scope. A name the successful
    // pattern did not bind is null. A single pattern keeps the names
    // `match_pat` stored and does not hide an outer binding.
    let mut names = Vec::new();
    if !alts.is_empty() {
        super::parse::bind_names(pattern, alts, &mut names);
    }
    let saved = line.get();
    let slot: RefCell<Option<WalkErr>> = RefCell::new(None);
    let mut bind_one = |value: &Value| -> Result<(), Failure> {
        for (index, pat) in pats.iter().enumerate() {
            line.set(saved);
            let last = index + 1 == pats.len();
            // `. as [$a]` takes the input apart. `.[0] as [$a]` does not:
            // the value was already reached by an index, so the pattern is
            // an invalid path.
            let bound_here = matches!(expr, Filter::Identity) && carrier_now(line, input_gen);
            if !bound_here && pattern_indexes(pat) {
                let err = destructure_near(pat, value, ctx);
                if err.stop.is_none() && !last {
                    continue;
                }
                *slot.borrow_mut() = Some(WalkErr::Path(Failure {
                    partial: Vec::new(),
                    message: err.message,
                    caught: err.caught,
                    stop: err.stop,
                }));
                return Ok(());
            }
            // `. as [$a,$b]` is already an invalid path. The slice in the
            // body must not replace the element that pattern names.
            if bound_here && pattern_indexes(pat) {
                let ident = Filter::Identity;
                let probe = Tail {
                    body: &ident,
                    orig_input: input,
                    orig_input_gen: input_gen,
                    names: &names,
                };
                match exec_pattern(
                    pat,
                    value,
                    path,
                    line,
                    true,
                    input_gen,
                    &[],
                    Vec::new(),
                    &probe,
                    ctx,
                ) {
                    Err(err) if err.stop.is_none() && !last => {
                        line.set(saved);
                        continue;
                    }
                    Err(err) => {
                        line.set(saved);
                        *slot.borrow_mut() = Some(WalkErr::Path(Failure {
                            partial: Vec::new(),
                            message: err.message,
                            caught: err.caught,
                            stop: err.stop,
                        }));
                        return Ok(());
                    }
                    Ok(_) => line.set(saved),
                }
            }
            let mut body_err: Option<WalkErr> = None;
            let matched = super::eval::match_pat(pat, value, ctx.env, &[], &mut |frame| {
                let owned;
                let frame = if names.is_empty() {
                    frame
                } else {
                    owned = null_names(frame, &names);
                    &owned
                };
                let child_env = child_scope(frame, ctx.env);
                let child = Ctx {
                    root: ctx.root,
                    next: ctx.next,
                    env: &child_env,
                    pbinds: ctx.pbinds,
                };
                // A plain `$name` keeps `.` on the input. Destructuring indexes
                // the value, so the body is no longer a path back to that input.
                let body_gen = if matches!(pat, Pattern::Var(_)) {
                    input_gen
                } else {
                    0
                };
                match send(body, input, path, line, body_gen, &child, emit) {
                    Ok(()) => Ok(()),
                    Err(err) => {
                        body_err = Some(err);
                        Err(fail(String::new()))
                    }
                }
            });
            if let Some(err) = body_err {
                // A path failure tries the next pattern. A write, a full
                // `first` or `limit`, and `halt` do not.
                let retry =
                    matches!(&err, WalkErr::Path(failure) if failure.stop.is_none()) && !last;
                if retry {
                    continue;
                }
                *slot.borrow_mut() = Some(err);
                return Ok(());
            }
            match matched {
                Ok(()) => return Ok(()),
                Err(failure) if failure.stop.is_some() || last => {
                    *slot.borrow_mut() = Some(WalkErr::Path(failure));
                    return Ok(());
                }
                Err(_) => {}
            }
        }
        Ok(())
    };
    let result = if matches!(expr, Filter::Identity) {
        bind_one(input)
    } else {
        stream::for_each_value(expr, input, ctx.env, |value| {
            bind_one(value)?;
            Ok(slot.borrow().is_some())
        })
        .map(|_| ())
    };
    if let Some(err) = slot.into_inner() {
        return Err(err);
    }
    match result {
        Ok(_) => Ok(()),
        Err(failure) => Err(WalkErr::Path(failure)),
    }
}

fn send_pipe(
    left: &Filter,
    right: &Filter,
    input: &Value,
    path: &[Step],
    line: &Cell<u64>,
    input_gen: u64,
    ctx: &Ctx,
    emit: &mut dyn FnMut(POut) -> Result<(), WalkErr>,
) -> Result<(), WalkErr> {
    let left_cell = Cell::new(line.get());
    send(
        left,
        input,
        path,
        &left_cell,
        input_gen,
        ctx,
        &mut |left_out| {
            let (gen, next_gen) = if left_out.carrier {
                (left_out.value_gen, left_out.value_gen)
            } else {
                (left_out.line, 0)
            };
            let cell = Cell::new(gen);
            send(
                right,
                &left_out.value,
                &left_out.path,
                &cell,
                next_gen,
                ctx,
                emit,
            )
        },
    )
}

/// `(expr)?` swallows a path failure and keeps places already written.
/// `expr?` directly on `]` does not: the flag is on the index or the slice.
fn send_optional(
    inner: &Filter,
    input: &Value,
    path: &[Step],
    line: &Cell<u64>,
    input_gen: u64,
    ctx: &Ctx,
    emit: &mut dyn FnMut(POut) -> Result<(), WalkErr>,
) -> Result<(), WalkErr> {
    if matches!(
        inner,
        Filter::At { optional: true, .. } | Filter::Slice { optional: true, .. }
    ) {
        return send(inner, input, path, line, input_gen, ctx, emit);
    }
    match send(inner, input, path, line, input_gen, ctx, emit) {
        Ok(()) => Ok(()),
        Err(WalkErr::Write(err)) => Err(WalkErr::Write(err)),
        Err(WalkErr::Enough) => Err(WalkErr::Enough),
        Err(WalkErr::Path(_)) => Ok(()),
    }
}

/// `first` and `limit` keep that many places and do not run a later bound.
fn send_take(
    inner: &Filter,
    want: usize,
    input: &Value,
    path: &[Step],
    line: &Cell<u64>,
    input_gen: u64,
    ctx: &Ctx,
    emit: &mut dyn FnMut(POut) -> Result<(), WalkErr>,
) -> Result<(), WalkErr> {
    if want == 0 {
        return Ok(());
    }
    let got = Cell::new(0usize);
    let result = send(inner, input, path, line, input_gen, ctx, &mut |out| {
        emit(out)?;
        got.set(got.get() + 1);
        if got.get() >= want {
            Err(WalkErr::Enough)
        } else {
            Ok(())
        }
    });
    match result {
        Ok(()) | Err(WalkErr::Enough) => Ok(()),
        Err(err) => Err(err),
    }
}

fn send_limit(
    count: &Filter,
    expr: &Filter,
    input: &Value,
    path: &[Step],
    line: &Cell<u64>,
    input_gen: u64,
    ctx: &Ctx,
    emit: &mut dyn FnMut(POut) -> Result<(), WalkErr>,
) -> Result<(), WalkErr> {
    let saved = line.get();
    let (counts, count_err) = parts(eval_in(count, input, ctx.env));
    line.set(saved);
    for count in &counts {
        line.set(saved);
        match super::stream::limit_how(count) {
            super::stream::LimitHow::None => {}
            super::stream::LimitHow::All => {
                send(expr, input, path, line, input_gen, ctx, emit)?;
            }
            super::stream::LimitHow::Count(need) => {
                send_take(expr, need, input, path, line, input_gen, ctx, emit)?;
            }
            super::stream::LimitHow::Subtract => {
                let got = take_limit_path(count, expr, input, path, line, input_gen, ctx)
                    .map_err(path_walk)?;
                for out in got {
                    emit(out)?;
                }
            }
        }
    }
    match count_err {
        Some(failure) => Err(WalkErr::Path(failure)),
        None => Ok(()),
    }
}

fn send_label(
    slot: u32,
    body: &Filter,
    input: &Value,
    path: &[Step],
    line: &Cell<u64>,
    input_gen: u64,
    ctx: &Ctx,
    emit: &mut dyn FnMut(POut) -> Result<(), WalkErr>,
) -> Result<(), WalkErr> {
    let (runtime, guard) = super::eval::LabelGuard::enter(slot);
    let result = send(body, input, path, line, input_gen, ctx, emit);
    drop(guard);
    match result {
        Ok(()) => Ok(()),
        Err(err) if walk_label_hit(&err, runtime) => Ok(()),
        Err(err) => Err(err),
    }
}

fn walk_label_hit(err: &WalkErr, runtime: u32) -> bool {
    let WalkErr::Path(failure) = err else {
        return false;
    };
    if failure.stop.is_some() {
        return false;
    }
    super::eval::is_label_hit(failure, runtime)
}

fn path_walk(err: PathErr) -> WalkErr {
    WalkErr::Path(Failure {
        partial: Vec::new(),
        message: err.message,
        caught: err.caught,
        stop: err.stop,
    })
}

fn send_call(
    func: &std::rc::Weak<super::FnCell>,
    args: &[Filter],
    input: &Value,
    path: &[Step],
    line: &Cell<u64>,
    input_gen: u64,
    ctx: &Ctx,
    emit: &mut dyn FnMut(POut) -> Result<(), WalkErr>,
) -> Result<(), WalkErr> {
    let Some(cell) = func.upgrade() else {
        return Err(WalkErr::Path(fail("function is not defined".to_string())));
    };
    let body = std::rc::Rc::clone(&cell.body.borrow());
    let saved = line.get();
    let slot: RefCell<Option<WalkErr>> = RefCell::new(None);
    let result = stream::each_binding(func, args, input, ctx.env, &mut |frame| {
        let _guard = super::func::set_scope(std::rc::Rc::clone(&frame.scope));
        let child = Ctx {
            root: ctx.root,
            next: ctx.next,
            env: &frame.env,
            pbinds: ctx.pbinds,
        };
        let branch = Cell::new(saved);
        match send(&body, input, path, &branch, input_gen, &child, emit) {
            Ok(()) => Ok(false),
            Err(err) => {
                *slot.borrow_mut() = Some(err);
                Ok(true)
            }
        }
    });
    if let Some(err) = slot.into_inner() {
        return Err(err);
    }
    match result {
        Ok(_) => Ok(()),
        Err(failure) => Err(WalkErr::Path(failure)),
    }
}

fn send_param(
    name: &str,
    input: &Value,
    path: &[Step],
    line: &Cell<u64>,
    input_gen: u64,
    ctx: &Ctx,
    emit: &mut dyn FnMut(POut) -> Result<(), WalkErr>,
) -> Result<(), WalkErr> {
    let Some(closure) = super::func::lookup(name) else {
        return Err(WalkErr::Path(fail(format!("{name} is not defined"))));
    };
    let _guard = super::func::set_scope(std::rc::Rc::clone(&closure.scope));
    let env = closure.env.flatten();
    let child = Ctx {
        root: ctx.root,
        next: ctx.next,
        env: &env,
        pbinds: ctx.pbinds,
    };
    send(&closure.expr, input, path, line, input_gen, &child, emit)
}

/// Places found by the ordinary path walk. Used when the filter is not a
/// slice or something wrapped around one. The whole filter still runs
/// before the first place is written.
fn emit_run(
    filter: &Filter,
    input: &Value,
    path: &[Step],
    line: &Cell<u64>,
    input_gen: u64,
    ctx: &Ctx,
    emit: &mut dyn FnMut(POut) -> Result<(), WalkErr>,
) -> Result<(), WalkErr> {
    match run(filter, input, path, line, input_gen, ctx) {
        Ok(outs) => {
            for out in outs {
                emit(out)?;
            }
            Ok(())
        }
        Err(err) => {
            for out in err.partial {
                emit(out)?;
            }
            Err(WalkErr::Path(Failure {
                partial: Vec::new(),
                message: err.message,
                caught: err.caught,
                stop: err.stop,
            }))
        }
    }
}

fn keep_walk(sent: Result<(), WalkErr>, slot: &RefCell<Option<WalkErr>>) -> Result<(), Failure> {
    match sent {
        Ok(()) => Ok(()),
        Err(err) => {
            let failure = match &err {
                WalkErr::Path(failure) | WalkErr::Write(failure) => failure.clone(),
                WalkErr::Enough => fail(String::new()),
            };
            *slot.borrow_mut() = Some(err);
            Err(failure)
        }
    }
}

/// Where the keys of one object live, so a `reduce` can set the next key
/// without scanning the pairs it has already stored.
pub(super) struct ObjIndex {
    ptr: usize,
    len: usize,
    map: HashMap<String, usize>,
}

/// `true` when `reduce` can apply `update` without building a fresh copy of
/// the accumulator. The shapes are `. + [expr]`, an assignment or `|=` whose
/// path starts at `.`, `setpath`, and a nest of arrays and objects that holds
/// the accumulator in one `.`.
pub(super) fn reduce_can_accelerate(update: &Filter) -> bool {
    match update {
        Filter::Bin {
            op: Op::Add,
            left,
            right,
        } if matches!(left.as_ref(), Filter::Identity) => is_array_term(right),
        Filter::Update {
            op: UpdateOp::Assign | UpdateOp::Modify,
            path,
            ..
        } => assignment_root(path),
        Filter::SetPath(_, _) => true,
        other => single_hole_wrap(other).is_some(),
    }
}

/// `foreach` emits one value per step for these. That value is the updated
/// accumulator, so the next step can continue from it.
pub(super) fn foreach_can_accelerate(update: &Filter) -> bool {
    match update {
        Filter::Bin {
            op: Op::Add,
            left,
            right,
        } if matches!(left.as_ref(), Filter::Identity) => is_array_term(right),
        Filter::Update {
            op: UpdateOp::Modify,
            path,
            ..
        } => assignment_root(path),
        _ => false,
    }
}

fn is_array_term(filter: &Filter) -> bool {
    matches!(
        filter,
        Filter::Collect(_) | Filter::Literal(Value::Array(_))
    )
}

fn assignment_root(path: &Filter) -> bool {
    match path {
        Filter::Field(_) => true,
        Filter::Index {
            at: ArrayIndex::At(_),
            ..
        } => true,
        Filter::At {
            base,
            optional: false,
            ..
        } => matches!(base.as_ref(), Filter::Identity),
        _ => false,
    }
}

/// Apply one `reduce` step in place. `Ok(false)` means the update is not one
/// of the shapes above, and nothing was run. The accumulator is the only
/// owner of its array or object, so appending and writing one key or index
/// reuse that buffer. A nest with one `.` moves that accumulator into place.
pub(super) fn accelerate_reduce(
    update: &Filter,
    acc: &mut Value,
    env: &[(String, Value)],
    lookup: &mut Option<ObjIndex>,
) -> Result<bool, Failure> {
    match update {
        Filter::Bin {
            op: Op::Add,
            left,
            right,
        } if matches!(left.as_ref(), Filter::Identity) => match right.as_ref() {
            Filter::Collect(inner) => {
                accelerate_append(inner, acc, env, lookup)?;
                Ok(true)
            }
            Filter::Literal(Value::Array(items)) => {
                extend_acc(acc, items.iter().cloned().collect(), lookup)?;
                Ok(true)
            }
            _ => Ok(false),
        },
        Filter::Update {
            op: UpdateOp::Assign,
            path,
            expr,
        } if assignment_root(path) => {
            accelerate_assign(path, expr, acc, env, lookup)?;
            Ok(true)
        }
        Filter::Update {
            op: UpdateOp::Modify,
            path,
            expr,
        } if assignment_root(path) => {
            accelerate_modify(path, expr, acc, env, lookup)?;
            Ok(true)
        }
        Filter::SetPath(path, value) => {
            accelerate_setpath(path, value, acc, env, lookup)?;
            Ok(true)
        }
        // `[.]` is the accumulator wrapped in one array. Moving it is the
        // same value as a copy, and a deep nest stays one step per wrap.
        Filter::Collect(inner) if matches!(inner.as_ref(), Filter::Identity) => {
            let inner = std::mem::replace(acc, Value::Null);
            *acc = Value::array(vec![inner]);
            *lookup = None;
            Ok(true)
        }
        // `{a:.}`, `[., 1]`, and `[[.]]` are the same move into a larger shell.
        // A second `.` stays here as `Ok(false)`, because that copy is real.
        other => match single_hole_wrap(other) {
            Some(shell) => {
                let mut carried = std::mem::replace(acc, Value::Null);
                *acc = fill_shell(&shell, &mut carried);
                *lookup = None;
                Ok(true)
            }
            None => Ok(false),
        },
    }
}

/// Arrays and objects around the one `.` in a `reduce` update.
/// A field, a lookup, a variable, a second `.`, or anything that can fail
/// stays out, so the copying evaluator still runs those.
enum Shell<'a> {
    Hole,
    Lit(&'a Value),
    Arr(Vec<Shell<'a>>),
    Obj(Vec<(&'a str, Shell<'a>)>),
}

fn single_hole_wrap(filter: &Filter) -> Option<Shell<'_>> {
    let mut holes = 0;
    let shell = compile_slot(filter, &mut holes)?;
    if holes == 1 {
        Some(shell)
    } else {
        None
    }
}

fn compile_slot<'a>(filter: &'a Filter, holes: &mut usize) -> Option<Shell<'a>> {
    match filter {
        Filter::Identity => {
            *holes += 1;
            if *holes > 1 {
                None
            } else {
                Some(Shell::Hole)
            }
        }
        Filter::Literal(value) => Some(Shell::Lit(value)),
        Filter::Collect(inner) => Some(Shell::Arr(compile_stream(inner, holes)?)),
        Filter::Object(fields) => {
            let mut pairs = Vec::with_capacity(fields.len());
            for field in fields {
                let ObjField::Pair { key, value } = field else {
                    return None;
                };
                let Filter::Literal(Value::String(name)) = key.as_ref() else {
                    return None;
                };
                pairs.push((name.as_str(), compile_slot(value, holes)?));
            }
            Some(Shell::Obj(pairs))
        }
        _ => None,
    }
}

/// The values inside `[ ]`, in order. A comma runs the left side, then the right.
fn compile_stream<'a>(filter: &'a Filter, holes: &mut usize) -> Option<Vec<Shell<'a>>> {
    match filter {
        Filter::Comma(left, right) => {
            let mut items = compile_stream(left, holes)?;
            items.extend(compile_stream(right, holes)?);
            Some(items)
        }
        other => Some(vec![compile_slot(other, holes)?]),
    }
}

fn fill_shell(shell: &Shell, hole: &mut Value) -> Value {
    match shell {
        Shell::Hole => std::mem::replace(hole, Value::Null),
        Shell::Lit(value) => (*value).clone(),
        Shell::Arr(items) => {
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                out.push(fill_shell(item, hole));
            }
            Value::array(out)
        }
        Shell::Obj(fields) => {
            let mut pairs = Vec::with_capacity(fields.len());
            for (name, slot) in fields {
                put(&mut pairs, (*name).to_string(), fill_shell(slot, hole));
            }
            Value::object(pairs)
        }
    }
}

/// `|=` reads the path first. The filter runs on the value at that path.
/// No output removes the value. One output is written back. One key or index
/// is written into the accumulator. Several places, or a slice, copy once.
fn accelerate_modify(
    path: &Filter,
    expr: &Filter,
    acc: &mut Value,
    env: &[(String, Value)],
    lookup: &mut Option<ObjIndex>,
) -> Result<(), Failure> {
    let paths = root_paths(path, acc, env)?;
    if paths.is_empty() {
        return Ok(());
    }
    if let [path] = paths.as_slice() {
        if let [step] = path.as_slice() {
            if matches!(step, Step::Key(_) | Step::Index { .. }) {
                return modify_one(acc, step, expr, env, lookup);
            }
        }
    }
    let values = modify(&paths, expr, acc, env)?;
    *acc = values.into_iter().next().unwrap_or(Value::Null);
    *lookup = None;
    Ok(())
}

fn modify_one(
    acc: &mut Value,
    step: &Step,
    expr: &Filter,
    env: &[(String, Value)],
    lookup: &mut Option<ObjIndex>,
) -> Result<(), Failure> {
    let path = vec![step.clone()];
    let current = value_at(acc, &path).map_err(fail)?;
    match eval_first(expr, &current, env)? {
        None => remove_simple(acc, step, lookup),
        Some(value) => write_simple(acc, &[path], value, lookup),
    }
}

fn remove_simple(
    acc: &mut Value,
    step: &Step,
    lookup: &mut Option<ObjIndex>,
) -> Result<(), Failure> {
    match step {
        Step::Key(key) => match acc {
            Value::Null => Ok(()),
            Value::Object(pairs) => {
                pairs.make_mut().retain(|(name, _)| name != key);
                *lookup = None;
                Ok(())
            }
            Value::Array(_) => Err(fail(format!("Cannot index array with string \"{key}\""))),
            other => Err(fail(format!(
                "Cannot index {} with string \"{key}\"",
                other.type_name()
            ))),
        },
        Step::Index { at, .. } => match acc {
            Value::Null => Ok(()),
            Value::String(_) => Err(fail("Cannot index string with number".to_string())),
            Value::Object(_) => Err(fail("Cannot index object with number".to_string())),
            Value::Array(items) => {
                let items = items.make_mut();
                if let Some(position) = resolve_index(*at, items.len()) {
                    if position < items.len() {
                        items.remove(position);
                    }
                }
                Ok(())
            }
            other => Err(fail(format!(
                "Cannot index {} with number",
                other.type_name()
            ))),
        },
        _ => unreachable!("simple path"),
    }
}

/// `setpath` runs the value first, then the path, both on the original
/// accumulator. `reduce` keeps the last write.
fn accelerate_setpath(
    path_f: &Filter,
    value_f: &Filter,
    acc: &mut Value,
    env: &[(String, Value)],
    lookup: &mut Option<ObjIndex>,
) -> Result<(), Failure> {
    let (values, value_err) = parts(eval_in(value_f, acc, env));
    if values.is_empty() {
        if let Some(failure) = value_err {
            return Err(failure);
        }
        *acc = Value::Null;
        *lookup = None;
        return Ok(());
    }
    // Each value is applied to the original accumulator. `reduce` keeps the
    // last one. A failure discards those writes, so the accumulator stays put
    // until the whole update succeeds.
    let keep = value_err.is_none();
    let mut last: Option<Value> = None;
    for (i, value) in values.iter().enumerate() {
        let (paths, path_err) = parts(eval_in(path_f, acc, env));
        let final_write = keep && i + 1 == values.len() && paths.len() == 1 && path_err.is_none();
        if final_write && last.is_none() {
            match set_inplace(acc, &paths[0], value, lookup) {
                Ok(true) => return Ok(()),
                Ok(false) => {}
                Err(failure) => return Err(failure),
            }
        }
        for path in &paths {
            let written = set_value(acc, path, value).map_err(fail)?;
            if keep {
                last = Some(written);
            }
        }
        if let Some(failure) = path_err {
            return Err(failure);
        }
    }
    if let Some(failure) = value_err {
        return Err(failure);
    }
    match last {
        Some(value) => {
            *acc = value;
            *lookup = None;
        }
        None => {
            *acc = Value::Null;
            *lookup = None;
        }
    }
    Ok(())
}

/// One `setpath` write onto the accumulator. `Ok(false)` means the path is
/// not a single key or index, and the accumulator was left as it was.
fn set_inplace(
    acc: &mut Value,
    path: &Value,
    value: &Value,
    lookup: &mut Option<ObjIndex>,
) -> Result<bool, Failure> {
    let Value::Array(keys) = path else {
        return Ok(false);
    };
    if keys.is_empty() {
        *acc = value.clone();
        *lookup = None;
        return Ok(true);
    }
    if keys.len() != 1 {
        return Ok(false);
    }
    match &keys[0] {
        Value::String(name) => {
            object_set(acc, name, value.clone(), lookup).map_err(fail)?;
            Ok(true)
        }
        Value::Number(text) => {
            let shown = canonical_number(text);
            let Some(index) = array_index(&shown) else {
                return match acc {
                    Value::Null | Value::Array(_) => Err(fail(huge_index_error(&shown))),
                    other => Err(fail(format!(
                        "Cannot index {} with number",
                        other.type_name()
                    ))),
                };
            };
            array_set(acc, index, value.clone()).map_err(fail)?;
            *lookup = None;
            Ok(true)
        }
        _ => Ok(false),
    }
}

fn accelerate_append(
    inner: &Filter,
    acc: &mut Value,
    env: &[(String, Value)],
    lookup: &mut Option<ObjIndex>,
) -> Result<(), Failure> {
    // `[expr]` is one array, or an error with nothing kept from `expr`.
    let items = match eval_in(inner, acc, env) {
        Ok(items) => items,
        Err(failure) => return Err(bare_fail(failure)),
    };
    extend_acc(acc, items, lookup)
}

fn extend_acc(
    acc: &mut Value,
    items: Vec<Value>,
    lookup: &mut Option<ObjIndex>,
) -> Result<(), Failure> {
    if matches!(acc, Value::Null) {
        *acc = Value::array(items);
        *lookup = None;
        return Ok(());
    }
    if let Value::Array(slot) = acc {
        slot.make_mut().extend(items);
        return Ok(());
    }
    let right = Value::array(items);
    match add_values(acc, &right) {
        Ok(value) => {
            *acc = value;
            *lookup = None;
            Ok(())
        }
        Err(message) => Err(fail(message)),
    }
}

fn accelerate_assign(
    path: &Filter,
    expr: &Filter,
    acc: &mut Value,
    env: &[(String, Value)],
    lookup: &mut Option<ObjIndex>,
) -> Result<(), Failure> {
    // `=` runs the right-hand side first. An empty stream yields nothing,
    // which `reduce` stores as null, and does not read the path.
    let (rhs, rhs_err) = parts(eval_in(expr, acc, env));
    if rhs.is_empty() {
        if let Some(failure) = rhs_err {
            return Err(failure);
        }
        *acc = Value::Null;
        *lookup = None;
        return Ok(());
    }
    let paths = root_paths(path, acc, env)?;
    if let Some(failure) = rhs_err {
        return Err(failure);
    }
    if paths.is_empty() {
        return Ok(());
    }
    let simple = paths
        .iter()
        .all(|path| matches!(path.as_slice(), [Step::Key(_)] | [Step::Index { .. }]));
    if simple {
        let value = rhs.into_iter().next_back().expect("value");
        return write_simple(acc, &paths, value, lookup);
    }
    // A slice or another compound path. Each value is tried against the
    // original accumulator, and `reduce` keeps only the last success.
    let last = rhs.len() - 1;
    for (i, value) in rhs.into_iter().enumerate() {
        match assign_one(&paths, acc, value) {
            Err(message) => return Err(fail(message)),
            Ok(produced) if i == last => {
                *acc = produced;
                *lookup = None;
            }
            Ok(_) => {}
        }
    }
    Ok(())
}

/// Places an assignment or `|=` writes, read before the write.
fn root_paths(
    path: &Filter,
    acc: &Value,
    env: &[(String, Value)],
) -> Result<Vec<Vec<Step>>, Failure> {
    match path {
        Filter::Field(name) => Ok(vec![vec![Step::Key(name.clone())]]),
        Filter::Index {
            at: ArrayIndex::At(index),
            text,
        } => Ok(vec![vec![Step::Index {
            at: *index,
            text: text.clone(),
        }]]),
        Filter::At { key, .. } => assignment_paths(acc, key, env),
        _ => unreachable!("assignment root"),
    }
}

/// Paths of `.[expr]`, read from the accumulator before any write.
fn assignment_paths(
    acc: &Value,
    key: &Filter,
    env: &[(String, Value)],
) -> Result<Vec<Vec<Step>>, Failure> {
    let (keys, key_err) = parts(eval_in(key, acc, env));
    if let Some(failure) = key_err {
        // A later failure in the key wins over an index error, and over a
        // failure that already happened on the right-hand side.
        return Err(failure);
    }
    let mut paths = Vec::with_capacity(keys.len());
    for key in &keys {
        match classify_step(acc, key) {
            Ok(step) => paths.push(vec![step]),
            Err(message) => return Err(fail(message)),
        }
    }
    Ok(paths)
}

/// The step a key will write, without reading the value already stored there.
fn classify_step(container: &Value, key: &Value) -> Result<Step, String> {
    match key {
        Value::String(name) => match container {
            Value::Null | Value::Object(_) => Ok(Step::Key(name.to_string())),
            other => Err(format!(
                "Cannot index {} with string \"{name}\"",
                other.type_name()
            )),
        },
        Value::Number(text) => {
            let shown = canonical_number(text);
            let Some(index) = array_index(&shown) else {
                return if matches!(container, Value::Null | Value::Array(_)) {
                    if matches!(crate::number::real(&shown), crate::number::Real::Nan) {
                        Err("Cannot set array element at NaN index".to_string())
                    } else if negative_huge(&shown) {
                        Ok(Step::Huge(shown))
                    } else {
                        Err(format!(
                            "Invalid path expression with result {}",
                            clipped(&Value::Null, 29)
                        ))
                    }
                } else {
                    Err(format!(
                        "Cannot index {} with number",
                        container.type_name()
                    ))
                };
            };
            match container {
                Value::Null | Value::Array(_) => Ok(Step::Index {
                    at: index,
                    text: shown,
                }),
                other => Err(format!("Cannot index {} with number", other.type_name())),
            }
        }
        _ => match real_step(container, key) {
            Ok(Real::Done(step, _)) => Ok(step),
            Ok(Real::Huge(text)) => {
                if matches!(crate::number::real(&text), crate::number::Real::Nan) {
                    Err("Cannot set array element at NaN index".to_string())
                } else {
                    Err(format!(
                        "Invalid path expression with result {}",
                        clipped(&Value::Null, 29)
                    ))
                }
            }
            Err(message) => Err(message),
        },
    }
}

fn write_simple(
    acc: &mut Value,
    paths: &[Vec<Step>],
    mut value: Value,
    lookup: &mut Option<ObjIndex>,
) -> Result<(), Failure> {
    let last = paths.len() - 1;
    for (i, path) in paths.iter().enumerate() {
        let piece = if i == last {
            std::mem::replace(&mut value, Value::Null)
        } else {
            value.clone()
        };
        match &path[0] {
            Step::Key(key) => object_set(acc, key, piece, lookup).map_err(fail)?,
            Step::Index { at, .. } => {
                array_set(acc, *at, piece).map_err(fail)?;
                *lookup = None;
            }
            _ => unreachable!("simple path"),
        }
    }
    Ok(())
}

fn object_set(
    acc: &mut Value,
    key: &str,
    value: Value,
    lookup: &mut Option<ObjIndex>,
) -> Result<(), String> {
    match acc {
        Value::Null => {
            *acc = Value::object(vec![(key.to_string(), value)]);
            let ptr = match acc {
                Value::Object(pairs) => pairs.as_ptr() as usize,
                _ => 0,
            };
            let mut map = HashMap::with_capacity(8);
            map.insert(key.to_string(), 0);
            *lookup = Some(ObjIndex { ptr, len: 1, map });
            return Ok(());
        }
        Value::Array(_) => {
            return Err(format!("Cannot index array with string \"{key}\""));
        }
        Value::Object(_) => {}
        other => {
            return Err(format!(
                "Cannot index {} with string \"{key}\"",
                other.type_name()
            ));
        }
    }
    let Value::Object(pairs) = acc else {
        unreachable!("object");
    };
    let pairs = pairs.make_mut();
    let ptr = pairs.as_ptr() as usize;
    let current = lookup
        .as_ref()
        .is_some_and(|index| index.ptr == ptr && index.len == pairs.len());
    if !current {
        let mut map = HashMap::with_capacity(pairs.len().saturating_mul(2).max(8));
        for (i, (name, _)) in pairs.iter().enumerate() {
            map.entry(name.clone()).or_insert(i);
        }
        *lookup = Some(ObjIndex {
            ptr,
            len: pairs.len(),
            map,
        });
    }
    let index = lookup.as_mut().expect("index");
    if let Some(&slot) = index.map.get(key) {
        pairs[slot].1 = value;
        return Ok(());
    }
    let slot = pairs.len();
    index.map.insert(key.to_string(), slot);
    pairs.push((key.to_string(), value));
    index.ptr = pairs.as_ptr() as usize;
    index.len = pairs.len();
    Ok(())
}

fn array_set(acc: &mut Value, index: i64, value: Value) -> Result<(), String> {
    if let Value::String(_) = acc {
        return Err("Cannot index string with number".to_string());
    }
    match acc {
        Value::Object(_) => return Err("Cannot index object with number".to_string()),
        Value::Null => *acc = Value::array(Vec::new()),
        Value::Array(_) => {}
        other => {
            return Err(format!("Cannot index {} with number", other.type_name()));
        }
    }
    let Value::Array(items) = acc else {
        unreachable!("array");
    };
    let items = items.make_mut();
    let Some(position) = resolve_index(index, items.len()) else {
        return Err("Out of bounds negative array index".to_string());
    };
    if position >= items.len() {
        items.resize(position + 1, Value::Null);
    }
    items[position] = value;
    Ok(())
}

fn assign_one(paths: &[Vec<Step>], input: &Value, value: Value) -> Result<Value, String> {
    let mut edit = Edit::Pure(input.clone());
    write_all(&mut edit, paths, &Write::Put(value))?;
    Ok(freeze(&edit))
}

/// `|=` keeps the first output of the filter at each place. No output removes it.
fn modify(
    paths: &[Vec<Step>],
    expr: &Filter,
    input: &Value,
    env: &[(String, Value)],
) -> Result<Vec<Value>, Failure> {
    let mut edit = Edit::Pure(input.clone());
    for path in paths {
        let current = read_edit(&edit, path).map_err(fail)?;
        let write = match eval_first(expr, &current, env)? {
            None => Write::Remove,
            Some(value) => Write::Put(value),
        };
        write_path(&mut edit, path, &write).map_err(fail)?;
    }
    Ok(vec![freeze(&edit)])
}

fn write_all(edit: &mut Edit, paths: &[Vec<Step>], write: &Write) -> Result<(), String> {
    for path in paths {
        write_path(edit, path, write)?;
    }
    Ok(())
}

/// The first output only. A later failure in a comma is not run once one
/// value has already been produced.
fn eval_first(
    filter: &Filter,
    input: &Value,
    env: &[(String, Value)],
) -> Result<Option<Value>, Failure> {
    match filter {
        Filter::Empty => Ok(None),
        Filter::Comma(left, right) => {
            if let Some(value) = eval_first(left, input, env)? {
                return Ok(Some(value));
            }
            eval_first(right, input, env)
        }
        Filter::Pipe(left, right) => {
            let produced = match eval_in(left, input, env) {
                Ok(values) => values,
                Err(failure) => {
                    for value in &failure.partial {
                        if let Some(hit) = eval_first(right, value, env)? {
                            return Ok(Some(hit));
                        }
                    }
                    return Err(bare_fail(failure));
                }
            };
            for value in &produced {
                if let Some(hit) = eval_first(right, value, env)? {
                    return Ok(Some(hit));
                }
            }
            Ok(None)
        }
        _ => match eval_in(filter, input, env) {
            Ok(values) => Ok(values.into_iter().next()),
            Err(failure) => Err(bare_fail(failure)),
        },
    }
}

/// One output of a path expression.
///
/// `carrier` means this value was reached by indexing the input, so a later
/// index extends `path`. `line` is the lineage after this output: an index
/// moves it, a constant does not. `and` and `or` read `line`; `|` reads
/// `carrier`. A matching `null`, `true`, or `false` is marked a carrier so
/// the next index is a real one.
#[derive(Clone)]
struct POut {
    value: Value,
    path: Vec<Step>,
    value_gen: u64,
    line: u64,
    carrier: bool,
    /// An index that does not fit. It must not compare equal to `null`.
    force_broken: bool,
}

struct PathErr {
    partial: Vec<POut>,
    message: String,
    caught: Option<Value>,
    stop: Option<Stop>,
}

/// A name bound while walking a path. The body loads it as that path.
#[derive(Clone)]
struct PBind {
    name: String,
    value: Value,
    path: Vec<Step>,
    value_gen: u64,
    line: u64,
    carrier: bool,
}

struct Ctx<'a> {
    root: &'a Value,
    next: &'a Cell<u64>,
    env: &'a [(String, Value)],
    pbinds: &'a [PBind],
}

struct Tail<'a> {
    body: &'a Filter,
    orig_input: &'a Value,
    orig_input_gen: u64,
    /// Every name bound by the pattern or a `?//` alternative.
    names: &'a [String],
}

#[derive(Clone)]
enum Job {
    Arr {
        pats: Vec<Pattern>,
        value: Value,
        carrier: bool,
        input_gen: u64,
        index: usize,
    },
    Obj {
        fields: Vec<PatField>,
        value: Value,
        carrier: bool,
        input_gen: u64,
        index: usize,
    },
}

struct Hit {
    value: Value,
    path: Vec<Step>,
    input_gen: u64,
}

enum Real {
    Done(Step, Value),
    /// A number too large to store. On `null` or an array this is not an error.
    /// The string is the spelling `path` prints.
    Huge(String),
}

fn classify_one(root: &Value, out: POut) -> Found {
    if out.force_broken {
        if out.path.iter().any(|step| match step {
            Step::Huge(text) => matches!(crate::number::real(text), crate::number::Real::Nan),
            _ => false,
        }) {
            return Found::NanIndex;
        }
        return Found::Broken(out.value);
    }
    if out.carrier {
        return Found::Path(out.path);
    }
    if matches!(out.value, Value::Null | Value::Bool(_)) {
        if let Ok(at) = value_at(root, &out.path) {
            if same_boolish(&out.value, &at) {
                return Found::Path(out.path);
            }
        }
    }
    Found::Broken(out.value)
}

fn same_boolish(left: &Value, right: &Value) -> bool {
    match (left, right) {
        (Value::Null, Value::Null) => true,
        (Value::Bool(left), Value::Bool(right)) => left == right,
        _ => false,
    }
}

fn run(
    filter: &Filter,
    input: &Value,
    path: &[Step],
    line: &Cell<u64>,
    input_gen: u64,
    ctx: &Ctx,
) -> Result<Vec<POut>, PathErr> {
    match filter {
        Filter::Identity => Ok(vec![pass(input, path, line, input_gen, ctx.root)]),
        Filter::Field(key) => run_field(key, input, path, line, input_gen, ctx),
        Filter::Index { at, text } => run_fixed_index(at, text, input, path, line, input_gen, ctx),
        Filter::GetPath(arg) => run_getpath(arg, input, path, line, input_gen, ctx),
        Filter::Slice {
            base,
            start,
            end,
            optional,
        } => run_slice(
            base, start, end, *optional, input, path, line, input_gen, ctx,
        ),
        Filter::Iterate => run_iterate(input, path, line, input_gen, ctx),
        Filter::Recurse => run_recurse(input, path, line, input_gen, ctx),
        Filter::RecurseDown { step, cond } => {
            run_recurse_down(step, cond.as_deref(), input, path, line, input_gen, ctx)
        }
        Filter::Optional(inner) => run_optional(inner, input, path, line, input_gen, ctx),
        Filter::Pipe(left, right) => run_pipe(left, right, input, path, line, input_gen, ctx),
        Filter::Comma(left, right) => run_comma(left, right, input, path, line, input_gen, ctx),
        Filter::Alt(left, right) => run_alt(left, right, input, path, line, input_gen, ctx),
        Filter::And(left, right) => run_bool(left, right, input, path, line, input_gen, ctx, false),
        Filter::Or(left, right) => run_bool(left, right, input, path, line, input_gen, ctx, true),
        Filter::If { cond, yes, no } => run_if(cond, yes, no, input, path, line, input_gen, ctx),
        Filter::Select(cond) => run_select(cond, input, path, line, input_gen, ctx),
        Filter::Bind {
            expr,
            pattern,
            alts,
            body,
        } => run_bind(expr, pattern, alts, body, input, path, line, input_gen, ctx),
        Filter::Label { slot, body } => run_label(*slot, body, input, path, line, input_gen, ctx),
        Filter::Break { slot, .. } => Err(path_break(*slot)),
        Filter::At {
            base,
            key,
            optional,
        } => run_at(base, key, *optional, input, path, line, input_gen, ctx),
        Filter::Var { name, .. } => match ctx.pbinds.iter().find(|bind| &bind.name == name) {
            Some(bind) => Ok(vec![emit_bind(bind, ctx.root)]),
            None => run_fallback(filter, input, path, line, ctx),
        },
        Filter::Try { body, handler } => {
            run_try(body, handler.as_deref(), input, path, line, input_gen, ctx)
        }
        // `first` and `nth` keep the path of the value they select. `last(expr)`
        // is the value itself, so it falls through and is not a path.
        Filter::First(None) => index_here(
            &Value::Number(crate::value::Text::from("0".to_string())),
            input,
            path,
            line,
            input_gen,
            ctx,
        ),
        Filter::Last(None) => index_here(
            &Value::Number(crate::value::Text::from("-1".to_string())),
            input,
            path,
            line,
            input_gen,
            ctx,
        ),
        Filter::First(Some(inner)) => run_take(inner, input, path, line, input_gen, ctx, 1),
        Filter::Nth { index, expr: None } => {
            run_nth_index(index, input, path, line, input_gen, ctx)
        }
        Filter::Nth {
            index,
            expr: Some(expr),
        } => run_nth_gen(index, expr, input, path, line, input_gen, ctx),
        Filter::Limit(count, expr) => run_limit(count, expr, input, path, line, input_gen, ctx),
        Filter::Define { rest, .. } => run(rest, input, path, line, input_gen, ctx),
        Filter::Call { func, args } => run_call(func, args, input, path, line, input_gen, ctx),
        Filter::Param(name) => run_param(name, input, path, line, input_gen, ctx),
        Filter::Unresolved { name, arity, .. } => {
            Err(bare(format!("{name}/{arity} is not defined")))
        }
        // `any` and `all` are paths when the value they stop on was reached
        // through the input. The value itself is computed by `eval_quant`.
        Filter::Quant { all, stream, cond } => {
            let expanded = super::builtin::expand_quant(*all, stream, cond);
            run(&expanded, input, path, line, input_gen, ctx)
        }
        Filter::IsEmpty(expr) => {
            let expanded = super::builtin::expand_isempty(expr);
            run(&expanded, input, path, line, input_gen, ctx)
        }
        // `map_values(f)` is `.[] |= f`. Inside `path` that update is the
        // first element of `[update, []]`, which is the failure jq reports.
        Filter::MapValues(expr) => {
            let update = Filter::Update {
                op: UpdateOp::Modify,
                path: Box::new(Filter::Iterate),
                expr: expr.clone(),
            };
            let pair = Filter::Comma(
                Box::new(update),
                Box::new(Filter::Literal(Value::array(Vec::new()))),
            );
            let indexed = Filter::At {
                base: Box::new(Filter::Collect(Box::new(pair))),
                key: Box::new(Filter::Literal(Value::Number(crate::value::Text::from(
                    "0".to_string(),
                )))),
                optional: false,
            };
            run(&indexed, input, path, line, input_gen, ctx)
        }
        // The extract is a path on the state. A computed state cannot be indexed.
        Filter::Foreach {
            stream,
            pattern,
            alts,
            init,
            update,
            extract,
        } => run_foreach(
            stream,
            pattern,
            alts,
            init,
            update,
            extract.as_deref(),
            input,
            path,
            line,
            input_gen,
            ctx,
        ),
        Filter::StreamBind { expr, name, body } => {
            run_stream_bind(expr, name, body, input, path, line, input_gen, ctx)
        }
        Filter::MatchImpl {
            regex,
            flags,
            testing,
        } => run_match_value(regex, flags, *testing, input, path, line, ctx),
        // The inside is walked before the array exists. A failure there, such
        // as iterating a match array, is the failure jq reports.
        Filter::Collect(inner) => run_collect(inner, input, path, line, input_gen, ctx),
        // The stream is walked before the reduction finishes. A match in it
        // fails at `.[]` instead of leaving the reduced value.
        Filter::Reduce {
            stream,
            pattern,
            alts,
            init,
            update,
        } => run_reduce_path(
            stream, pattern, alts, init, update, input, path, line, input_gen, ctx,
        ),
        // `path(f |= g)` is `path([f |= g, []][0])`. The same is true of
        // `+=` and the other modify operators. `=` stays the updated value.
        Filter::Update { op, .. } if *op != UpdateOp::Assign => {
            run_modify_path(filter, input, path, line, ctx)
        }
        other => run_fallback(other, input, path, line, ctx),
    }
}

/// `path` rejected a value, or a filter raised an error. `(expr)?` swallows
/// the filter error and leaves the rejection, so a later slice bound still
/// does not run.
enum PathStop {
    Filter(Failure),
    Rejected(Failure),
}

fn stop_err(stop: PathStop) -> PathErr {
    let failure = match stop {
        PathStop::Filter(failure) | PathStop::Rejected(failure) => failure,
    };
    PathErr {
        partial: Vec::new(),
        message: failure.message,
        caught: failure.caught,
        stop: failure.stop,
    }
}

/// `foreach` in a path. A stream of plain values leaves the init's path in
/// place, so `foreach (0,1)` yields it. Iterating the input, as in `.[]`,
/// does not. An index in the init moves the lineage, and a later `.[]`
/// cannot iterate. The value after an update stays a path only when that
/// update put it back on the same path.
fn run_foreach(
    stream: &Filter,
    pattern: &Pattern,
    alts: &[Pattern],
    init: &Filter,
    update: &Filter,
    extract: Option<&Filter>,
    input: &Value,
    path: &[Step],
    line: &Cell<u64>,
    input_gen: u64,
    ctx: &Ctx,
) -> Result<Vec<POut>, PathErr> {
    let identity = Filter::Identity;
    let extract = extract.unwrap_or(&identity);
    let init_line = Cell::new(line.get());
    let inits = run(init, input, path, &init_line, input_gen, ctx)?;
    // `.[1], .` indexes on one side. The stream then refuses to iterate,
    // even though the other side is still the input. A constant init does
    // not move the lineage.
    if inits.iter().any(|out| !same_steps(&out.path, path)) {
        line.set(alloc(ctx.next));
    }
    let mut outputs = Vec::new();
    for (index, init_state) in inits.into_iter().enumerate() {
        let mut state = init_state;
        let null_input = Value::Null;
        let stream_input = if index == 0 { input } else { &null_input };
        let start_line = line.get();
        let slot: RefCell<Option<PathErr>> = RefCell::new(None);
        let streamed = stream_paths(
            stream,
            stream_input,
            path,
            line,
            input_gen,
            ctx,
            &mut |item| {
                if slot.borrow().is_some() {
                    return Ok(());
                }
                // A plain value keeps this path. An index moves the line or
                // the path, including `.[0] | 0`, and the update is no longer
                // a path back to the input. Later stream values are not run.
                let breaks = item.line != start_line || !same_steps(&item.path, path);
                match foreach_yield(
                    &mut state,
                    &item.value,
                    item.carrier,
                    breaks,
                    pattern,
                    alts,
                    update,
                    extract,
                    ctx,
                    &mut outputs,
                ) {
                    Ok(()) => Ok(()),
                    Err(err) => {
                        *slot.borrow_mut() = Some(err);
                        Err(PathStop::Filter(fail(String::new())))
                    }
                }
            },
        );
        if let Some(err) = slot.into_inner() {
            return Err(attach(outputs, err));
        }
        if let Err(stop) = streamed {
            return Err(attach(outputs, stop_err(stop)));
        }
    }
    Ok(outputs)
}

fn foreach_yield(
    state: &mut POut,
    item: &Value,
    item_carrier: bool,
    breaks: bool,
    pattern: &Pattern,
    alts: &[Pattern],
    update: &Filter,
    extract: &Filter,
    ctx: &Ctx,
    outputs: &mut Vec<POut>,
) -> Result<(), PathErr> {
    let mut pats = Vec::with_capacity(1 + alts.len());
    pats.push(pattern);
    pats.extend(alts);
    let mut names = Vec::new();
    if !alts.is_empty() {
        super::parse::bind_names(pattern, alts, &mut names);
    }
    for (index, pat) in pats.iter().enumerate() {
        let last = index + 1 == pats.len();
        // `.[] as [$a]` indexes a value reached through the input. A plain
        // `1` does not, so taking it apart is an invalid path and `?//` runs.
        if !item_carrier && pattern_indexes(pat) {
            let err = destructure_near(pat, item, ctx);
            if err.stop.is_none() && !last {
                continue;
            }
            return Err(err);
        }
        match foreach_one(
            state, item, pat, &names, breaks, update, extract, ctx, outputs,
        ) {
            Ok(()) => return Ok(()),
            Err(err) if err.stop.is_none() && !last => {}
            Err(err) => return Err(err),
        }
    }
    Ok(())
}

fn foreach_one(
    state: &mut POut,
    item: &Value,
    pattern: &Pattern,
    names: &[String],
    breaks: bool,
    update: &Filter,
    extract: &Filter,
    ctx: &Ctx,
    outputs: &mut Vec<POut>,
) -> Result<(), PathErr> {
    let mut step_err: Option<PathErr> = None;
    let matched = super::eval::match_pat(pattern, item, ctx.env, &[], &mut |frame| {
        if step_err.is_some() {
            return Ok(());
        }
        let owned;
        let frame = if names.is_empty() {
            frame
        } else {
            owned = null_names(frame, names);
            &owned
        };
        let child_env = child_scope(frame, ctx.env);
        let child = Ctx {
            root: ctx.root,
            next: ctx.next,
            env: &child_env,
            pbinds: ctx.pbinds,
        };
        let gen = if !breaks && state.carrier {
            state.value_gen
        } else {
            0
        };
        let cell = Cell::new(gen);
        let updates = match run(update, &state.value, &state.path, &cell, gen, &child) {
            Ok(updates) => updates,
            Err(err) => {
                step_err = Some(err);
                return Ok(());
            }
        };
        if updates.is_empty() {
            *state = constant(Value::Null, &state.path, state.line, ctx.root);
            return Ok(());
        }
        // Every update of this step was run on the accumulator as it stood.
        // A later one must not be judged against an accumulator this loop
        // already replaced.
        let origin = state.clone();
        for upd in &updates {
            let ext_gen = if upd.carrier { upd.value_gen } else { 0 };
            let ext_cell = Cell::new(ext_gen);
            let extracted = match run(extract, &upd.value, &upd.path, &ext_cell, ext_gen, &child) {
                Ok(more) => more,
                Err(err) => {
                    step_err = Some(err);
                    return Ok(());
                }
            };
            for one in extracted {
                if one.carrier {
                    outputs.push(one);
                } else {
                    step_err = Some(bare(format!(
                        "Invalid path expression with result {}",
                        clipped(&one.value, 29)
                    )));
                    return Ok(());
                }
            }
            if upd.carrier && same_steps(&upd.path, &origin.path) {
                *state = upd.clone();
            } else if slice_kept_array(&origin, upd) {
                *state = origin.clone();
            } else {
                *state = constant(upd.value.clone(), &origin.path, origin.line, ctx.root);
            }
        }
        Ok(())
    });
    if let Some(err) = step_err {
        return Err(err);
    }
    match matched {
        Ok(()) => Ok(()),
        Err(failure) => Err(PathErr {
            partial: Vec::new(),
            message: failure.message,
            caught: failure.caught,
            stop: failure.stop,
        }),
    }
}

fn run_call(
    func: &std::rc::Weak<super::FnCell>,
    args: &[Filter],
    input: &Value,
    path: &[Step],
    line: &Cell<u64>,
    input_gen: u64,
    ctx: &Ctx,
) -> Result<Vec<POut>, PathErr> {
    let Some(cell) = func.upgrade() else {
        return Err(bare("function is not defined".to_string()));
    };
    let body = std::rc::Rc::clone(&cell.body.borrow());
    let saved = line.get();
    let mut out = Vec::new();
    let result = stream::each_binding(func, args, input, ctx.env, &mut |frame| {
        let _guard = super::func::set_scope(std::rc::Rc::clone(&frame.scope));
        let child = Ctx {
            root: ctx.root,
            next: ctx.next,
            env: &frame.env,
            pbinds: ctx.pbinds,
        };
        let branch = Cell::new(saved);
        match run(&body, input, path, &branch, input_gen, &child) {
            Ok(more) => {
                out.extend(more);
                Ok(false)
            }
            Err(err) => {
                out.extend(err.partial);
                Err(Failure {
                    partial: Vec::new(),
                    message: err.message,
                    caught: err.caught,
                    stop: err.stop,
                })
            }
        }
    });
    match result {
        Ok(_) => Ok(out),
        Err(failure) => Err(PathErr {
            partial: out,
            message: failure.message,
            caught: failure.caught,
            stop: failure.stop,
        }),
    }
}

fn run_param(
    name: &str,
    input: &Value,
    path: &[Step],
    line: &Cell<u64>,
    input_gen: u64,
    ctx: &Ctx,
) -> Result<Vec<POut>, PathErr> {
    let Some(closure) = super::func::lookup(name) else {
        return Err(bare(format!("{name} is not defined")));
    };
    let _guard = super::func::set_scope(std::rc::Rc::clone(&closure.scope));
    let env = closure.env.flatten();
    let child = Ctx {
        root: ctx.root,
        next: ctx.next,
        env: &env,
        pbinds: ctx.pbinds,
    };
    run(&closure.expr, input, path, line, input_gen, &child)
}

/// The first `want` path outputs. A `comma` does not run its right side once
/// `want` outputs are in hand, so `first(.[], error("z"))` does not raise.
fn run_take(
    filter: &Filter,
    input: &Value,
    path: &[Step],
    line: &Cell<u64>,
    input_gen: u64,
    ctx: &Ctx,
    want: usize,
) -> Result<Vec<POut>, PathErr> {
    if want == 0 {
        return Ok(Vec::new());
    }
    match filter {
        Filter::Comma(left, right) => {
            let saved = line.get();
            let left_cell = Cell::new(saved);
            let mut out = run_take(left, input, path, &left_cell, input_gen, ctx, want)?;
            if out.len() >= want {
                out.truncate(want);
                return Ok(out);
            }
            let right_cell = Cell::new(saved);
            let more = run_take(
                right,
                input,
                path,
                &right_cell,
                input_gen,
                ctx,
                want - out.len(),
            )?;
            out.extend(more);
            Ok(out)
        }
        Filter::Pipe(left, right) => {
            run_take_pipe(left, right, input, path, line, input_gen, ctx, want)
        }
        Filter::Empty => Ok(Vec::new()),
        Filter::First(Some(inner)) => {
            run_take(inner, input, path, line, input_gen, ctx, 1.min(want))
        }
        other => match run(other, input, path, line, input_gen, ctx) {
            Ok(mut outs) => {
                if outs.len() > want {
                    outs.truncate(want);
                }
                Ok(outs)
            }
            Err(err) if err.partial.len() >= want => {
                let mut outs = err.partial;
                outs.truncate(want);
                Ok(outs)
            }
            Err(err) => Err(err),
        },
    }
}

fn run_take_pipe(
    left: &Filter,
    right: &Filter,
    input: &Value,
    path: &[Step],
    line: &Cell<u64>,
    input_gen: u64,
    ctx: &Ctx,
    want: usize,
) -> Result<Vec<POut>, PathErr> {
    if let Filter::Comma(a, b) = left {
        let saved = line.get();
        let left_cell = Cell::new(saved);
        let mut out = run_take_pipe(a, right, input, path, &left_cell, input_gen, ctx, want)?;
        if out.len() >= want {
            out.truncate(want);
            return Ok(out);
        }
        let right_cell = Cell::new(saved);
        let more = run_take_pipe(
            b,
            right,
            input,
            path,
            &right_cell,
            input_gen,
            ctx,
            want - out.len(),
        )?;
        out.extend(more);
        return Ok(out);
    }
    let lefts = run(left, input, path, line, input_gen, ctx)?;
    let mut out = Vec::new();
    for left_out in lefts {
        if out.len() >= want {
            break;
        }
        let (gen, next_gen) = if left_out.carrier {
            (left_out.value_gen, left_out.value_gen)
        } else {
            (left_out.line, 0)
        };
        let cell = Cell::new(gen);
        let more = run_take(
            right,
            &left_out.value,
            &left_out.path,
            &cell,
            next_gen,
            ctx,
            want - out.len(),
        )?;
        out.extend(more);
    }
    if out.len() > want {
        out.truncate(want);
    }
    Ok(out)
}

fn run_nth_index(
    index: &Filter,
    input: &Value,
    path: &[Step],
    line: &Cell<u64>,
    input_gen: u64,
    ctx: &Ctx,
) -> Result<Vec<POut>, PathErr> {
    let saved = line.get();
    let (keys, key_err) = parts(eval_in(index, input, ctx.env));
    line.set(saved);
    let mut out = Vec::new();
    for key in &keys {
        line.set(saved);
        match index_here(key, input, path, line, input_gen, ctx) {
            Ok(hits) => out.extend(hits),
            Err(err) => return Err(attach(out, err)),
        }
    }
    finish_outs(out, key_err)
}

fn run_nth_gen(
    index: &Filter,
    expr: &Filter,
    input: &Value,
    path: &[Step],
    line: &Cell<u64>,
    input_gen: u64,
    ctx: &Ctx,
) -> Result<Vec<POut>, PathErr> {
    let saved = line.get();
    let (indexes, index_err) = parts(eval_in(index, input, ctx.env));
    line.set(saved);
    let mut out = Vec::new();
    for index in &indexes {
        line.set(saved);
        match take_nth_path(index, expr, input, path, line, input_gen, ctx) {
            Ok(more) => out.extend(more),
            Err(err) => return Err(attach(out, err)),
        }
    }
    finish_outs(out, index_err)
}

fn take_nth_path(
    index: &Value,
    expr: &Filter,
    input: &Value,
    path: &[Step],
    line: &Cell<u64>,
    input_gen: u64,
    ctx: &Ctx,
) -> Result<Vec<POut>, PathErr> {
    match stream::nth_span(index) {
        NthSpan::Negative => Err(bare("nth doesn't support negative indices".to_string())),
        NthSpan::Add(message) => Err(bare(message)),
        NthSpan::Take(0) => Ok(Vec::new()),
        NthSpan::Never => {
            run(expr, input, path, line, input_gen, ctx)?;
            Ok(Vec::new())
        }
        NthSpan::Take(need) => {
            let outs = run_take(expr, input, path, line, input_gen, ctx, need)?;
            if outs.len() < need {
                Ok(Vec::new())
            } else {
                Ok(vec![outs.into_iter().next_back().expect("need outputs")])
            }
        }
    }
}

fn run_limit(
    count: &Filter,
    expr: &Filter,
    input: &Value,
    path: &[Step],
    line: &Cell<u64>,
    input_gen: u64,
    ctx: &Ctx,
) -> Result<Vec<POut>, PathErr> {
    let saved = line.get();
    let (counts, count_err) = parts(eval_in(count, input, ctx.env));
    line.set(saved);
    let mut out = Vec::new();
    for count in &counts {
        line.set(saved);
        match take_limit_path(count, expr, input, path, line, input_gen, ctx) {
            Ok(more) => out.extend(more),
            Err(err) => return Err(attach(out, err)),
        }
    }
    finish_outs(out, count_err)
}

fn take_limit_path(
    count: &Value,
    expr: &Filter,
    input: &Value,
    path: &[Step],
    line: &Cell<u64>,
    input_gen: u64,
    ctx: &Ctx,
) -> Result<Vec<POut>, PathErr> {
    match stream::limit_how(count) {
        LimitHow::None => Ok(Vec::new()),
        LimitHow::All => run(expr, input, path, line, input_gen, ctx),
        LimitHow::Count(need) => run_take(expr, input, path, line, input_gen, ctx, need),
        LimitHow::Subtract => match run_take(expr, input, path, line, input_gen, ctx, 1) {
            Err(err) => Err(err),
            Ok(vals) if vals.is_empty() => Ok(Vec::new()),
            Ok(_) => match sub_values(
                count,
                &Value::Number(crate::value::Text::from("1".to_string())),
            ) {
                Err(message) => Err(bare(message)),
                Ok(_) => Ok(Vec::new()),
            },
        },
    }
}

fn carrier_now(line: &Cell<u64>, input_gen: u64) -> bool {
    input_gen != 0 && line.get() == input_gen
}

fn alloc(next: &Cell<u64>) -> u64 {
    let gen = next.get();
    next.set(gen + 1);
    gen
}

fn taken(value: Value, path: Vec<Step>, line: &Cell<u64>, next: &Cell<u64>) -> POut {
    let gen = alloc(next);
    line.set(gen);
    POut {
        value,
        path,
        value_gen: gen,
        line: gen,
        carrier: true,
        force_broken: false,
    }
}

fn pass(value: &Value, path: &[Step], line: &Cell<u64>, input_gen: u64, root: &Value) -> POut {
    let carrier = carrier_now(line, input_gen);
    let mut out = POut {
        value: value.clone(),
        path: path.to_vec(),
        value_gen: if carrier { line.get() } else { 0 },
        line: line.get(),
        carrier,
        force_broken: false,
    };
    if !carrier {
        upgrade(&mut out, root);
    }
    out
}

fn constant(value: Value, path: &[Step], lineage: u64, root: &Value) -> POut {
    let mut out = POut {
        value,
        path: path.to_vec(),
        value_gen: 0,
        line: lineage,
        carrier: false,
        force_broken: false,
    };
    upgrade(&mut out, root);
    out
}

fn upgrade(out: &mut POut, root: &Value) {
    if out.force_broken || out.carrier {
        return;
    }
    if !matches!(out.value, Value::Null | Value::Bool(_)) {
        return;
    }
    let Ok(at) = value_at(root, &out.path) else {
        return;
    };
    if same_boolish(&out.value, &at) {
        out.carrier = true;
        out.value_gen = if out.line == 0 { 1 } else { out.line };
    }
}

fn bare(message: String) -> PathErr {
    PathErr {
        partial: Vec::new(),
        message,
        caught: None,
        stop: None,
    }
}

fn attach(mut done: Vec<POut>, err: PathErr) -> PathErr {
    done.extend(err.partial);
    PathErr {
        partial: done,
        message: err.message,
        caught: err.caught,
        stop: err.stop,
    }
}

fn split_err(mut err: PathErr) -> (Vec<POut>, Failure) {
    (
        std::mem::take(&mut err.partial),
        Failure {
            partial: Vec::new(),
            message: err.message,
            caught: err.caught,
            stop: err.stop,
        },
    )
}

/// One argument of `match`, `test`, or `capture`. The next value is not read
/// when the body fails, which is how `path` stops on the match array.
fn run_stream_bind(
    expr: &Filter,
    name: &str,
    body: &Filter,
    input: &Value,
    path: &[Step],
    line: &Cell<u64>,
    input_gen: u64,
    ctx: &Ctx,
) -> Result<Vec<POut>, PathErr> {
    let mut out = Vec::new();
    let mut path_err: Option<PathErr> = None;
    let result = stream::for_each_value(expr, input, ctx.env, |value| {
        let child_env = child_scope(&[(name.to_string(), value.clone())], ctx.env);
        let child = Ctx {
            root: ctx.root,
            next: ctx.next,
            env: &child_env,
            pbinds: ctx.pbinds,
        };
        match run(body, input, path, line, input_gen, &child) {
            Ok(more) => {
                out.extend(more);
                Ok(false)
            }
            Err(err) => {
                path_err = Some(attach(std::mem::take(&mut out), err));
                Ok(true)
            }
        }
    });
    if let Some(err) = path_err {
        return Err(err);
    }
    match result {
        Ok(_) => Ok(out),
        Err(failure) => Err(path_from(out, failure)),
    }
}

/// `match` is a search piped into `.[]`. Iterate the array before the next
/// pattern runs, so a path error names that array.
fn run_collect(
    inner: &Filter,
    input: &Value,
    path: &[Step],
    line: &Cell<u64>,
    input_gen: u64,
    ctx: &Ctx,
) -> Result<Vec<POut>, PathErr> {
    let items = match run(inner, input, path, line, input_gen, ctx) {
        Ok(outs) => outs.into_iter().map(|out| out.value).collect(),
        Err(err) => return Err(bare_path(err)),
    };
    Ok(vec![constant(
        Value::array(items),
        path,
        line.get(),
        ctx.root,
    )])
}

/// `reduce` is the init's path when the stream is empty, or when every step
/// leaves that same path in place. Anything else is the reduced value.
fn kept_place(
    init: &POut,
    items: &[POut],
    pattern: &Pattern,
    alts: &[Pattern],
    update: &Filter,
    ctx: &Ctx,
) -> Option<POut> {
    if items.is_empty() {
        return Some(init.clone());
    }
    if !init.carrier || !alts.is_empty() || !keeps_place(update) {
        return None;
    }
    let mut acc = init.clone();
    for item in items {
        // A constant is not a path, so taking it apart is not this place.
        if !item.carrier && pattern_indexes(pattern) {
            return None;
        }
        let mut step: Option<Vec<POut>> = None;
        let matched = super::eval::match_pat(pattern, &item.value, ctx.env, &[], &mut |frame| {
            let child_env = child_scope(frame, ctx.env);
            let child = Ctx {
                root: ctx.root,
                next: ctx.next,
                env: &child_env,
                pbinds: ctx.pbinds,
            };
            let line = Cell::new(acc.line);
            let gen = if acc.carrier { acc.value_gen } else { 0 };
            match run(update, &acc.value, &acc.path, &line, gen, &child) {
                Ok(outs) => {
                    step = Some(outs);
                    Ok(())
                }
                Err(_) => Err(fail(String::new())),
            }
        });
        if matched.is_err() {
            return None;
        }
        let outs = step?;
        let [one] = outs.as_slice() else {
            return None;
        };
        if !(one.carrier && same_steps(&one.path, &acc.path)) {
            return None;
        }
        acc = one.clone();
    }
    Some(acc)
}

/// Steps that put the accumulator back where it was, so `reduce` can be a path.
fn keeps_place(filter: &Filter) -> bool {
    match filter {
        Filter::Identity | Filter::Select(_) => true,
        Filter::Pipe(left, right)
        | Filter::Alt(left, right)
        | Filter::And(left, right)
        | Filter::Or(left, right) => keeps_place(left) && keeps_place(right),
        Filter::Optional(inner) | Filter::Define { rest: inner, .. } => keeps_place(inner),
        Filter::Label { body, .. } => keeps_place(body),
        Filter::Try { body, handler } => {
            keeps_place(body) && handler.as_deref().is_none_or(keeps_place)
        }
        Filter::If { yes, no, .. } => keeps_place(yes) && keeps_place(no),
        Filter::Bind {
            pattern,
            alts,
            body,
            ..
        } => alts.is_empty() && matches!(pattern, Pattern::Var(_)) && keeps_place(body),
        _ => false,
    }
}

/// A slice of a non-empty array that still holds every element leaves the
/// accumulator on `before`. `foreach` has already yielded `after`'s path.
/// An empty array and a string stay results even when the slice spans them.
fn slice_kept_array(before: &POut, after: &POut) -> bool {
    let Value::Array(items) = &before.value else {
        return false;
    };
    let len = items.len();
    if len == 0 || after.path.len() <= before.path.len() {
        return false;
    }
    if !same_steps(&before.path, &after.path[..before.path.len()]) {
        return false;
    }
    after.path[before.path.len()..].iter().all(|step| {
        let Step::Slice { start, end } = step else {
            return false;
        };
        let (Ok(start), Ok(end)) = (bound_text(start), bound_text(end)) else {
            return false;
        };
        let (from, to) = slice_range(start.as_deref(), end.as_deref(), len);
        from == 0 && to == len
    })
}

fn same_steps(left: &[Step], right: &[Step]) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right)
            .all(|(one, other)| same_step(one, other))
}

fn same_step(left: &Step, right: &Step) -> bool {
    match (left, right) {
        (Step::Key(left), Step::Key(right)) => left == right,
        (Step::Index { at: left, .. }, Step::Index { at: right, .. }) => left == right,
        (Step::Huge(left), Step::Huge(right)) => left == right,
        (
            Step::Slice {
                start: left_start,
                end: left_end,
            },
            Step::Slice {
                start: right_start,
                end: right_end,
            },
        ) => {
            super::value::cmp_value(left_start, right_start) == 0
                && super::value::cmp_value(left_end, right_end) == 0
        }
        (Step::Raw(left), Step::Raw(right)) => super::value::cmp_value(left, right) == 0,
        _ => false,
    }
}

fn run_reduce_path(
    stream: &Filter,
    pattern: &Pattern,
    alts: &[Pattern],
    init: &Filter,
    update: &Filter,
    input: &Value,
    path: &[Step],
    line: &Cell<u64>,
    input_gen: u64,
    ctx: &Ctx,
) -> Result<Vec<POut>, PathErr> {
    let inits = match run(init, input, path, line, input_gen, ctx) {
        Ok(outs) => outs,
        Err(err) => return Err(bare_path(err)),
    };
    if inits.is_empty() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for (index, init_out) in inits.into_iter().enumerate() {
        let null_input = Value::Null;
        let stream_input = if index == 0 { input } else { &null_input };
        let start_line = line.get();
        let items = match run(stream, stream_input, path, line, input_gen, ctx) {
            Ok(outs) => outs,
            Err(err) => return Err(bare_path(err)),
        };
        if let Some(kept) = kept_place(&init_out, &items, pattern, alts, update, ctx) {
            out.push(kept);
            continue;
        }
        // A plain stream leaves the init in place for the first step, so a
        // later step names the element of the value that step was given.
        match reduce_off_path(
            &init_out, &items, start_line, pattern, alts, update, path, ctx,
        ) {
            Ok(place) => out.push(place),
            Err(err) => return Err(err),
        }
    }
    Ok(out)
}

/// A failure inside `[]` or `reduce` replaces the value being built.
fn bare_path(err: PathErr) -> PathErr {
    PathErr {
        partial: Vec::new(),
        message: err.message,
        caught: err.caught,
        stop: err.stop,
    }
}

/// One `reduce` whose steps leave the path. A stream value reached by
/// indexing makes the step a non-carrier, so the index names the element.
/// A plain value does not, so the first step reads the init and a later
/// step reads the value that previous step produced.
fn reduce_off_path(
    init: &POut,
    items: &[POut],
    start_line: u64,
    pattern: &Pattern,
    alts: &[Pattern],
    update: &Filter,
    path: &[Step],
    ctx: &Ctx,
) -> Result<POut, PathErr> {
    let mut names = Vec::new();
    super::parse::bind_names(pattern, alts, &mut names);
    let mut pats = Vec::with_capacity(1 + alts.len());
    pats.push(pattern);
    pats.extend(alts);
    let mut acc = init.clone();
    for item in items {
        let breaks = item.line != start_line || !same_steps(&item.path, path);
        let mut stepped = false;
        for (index, pat) in pats.iter().enumerate() {
            let last = index + 1 == pats.len();
            if !item.carrier && pattern_indexes(pat) {
                let err = destructure_near(pat, &item.value, ctx);
                if err.stop.is_none() && !last {
                    continue;
                }
                return Err(err);
            }
            let mut body_err: Option<PathErr> = None;
            let mut produced: Option<Vec<POut>> = None;
            let matched = super::eval::match_pat(pat, &item.value, ctx.env, &[], &mut |frame| {
                let frame = null_names(frame, &names);
                let child_env = child_scope(&frame, ctx.env);
                let child = Ctx {
                    root: ctx.root,
                    next: ctx.next,
                    env: &child_env,
                    pbinds: ctx.pbinds,
                };
                let gen = if !breaks && acc.carrier {
                    acc.value_gen
                } else {
                    0
                };
                let cell = Cell::new(gen);
                match run(update, &acc.value, &acc.path, &cell, gen, &child) {
                    Ok(outs) => {
                        produced = Some(outs);
                        Ok(())
                    }
                    Err(err) => {
                        body_err = Some(err);
                        Err(fail(String::new()))
                    }
                }
            });
            if let Some(err) = body_err {
                if err.stop.is_none() && !last {
                    continue;
                }
                return Err(err);
            }
            match matched {
                Ok(()) => {
                    acc = match produced.unwrap_or_default().pop() {
                        Some(upd) if upd.carrier && same_steps(&upd.path, &acc.path) => upd,
                        Some(upd) if slice_kept_array(&acc, &upd) => acc.clone(),
                        Some(upd) => constant(upd.value, &acc.path, acc.line, ctx.root),
                        None => constant(Value::Null, &acc.path, acc.line, ctx.root),
                    };
                    stepped = true;
                    break;
                }
                Err(failure) if failure.stop.is_some() || last => {
                    return Err(PathErr {
                        partial: Vec::new(),
                        message: failure.message,
                        caught: failure.caught,
                        stop: failure.stop,
                    });
                }
                Err(_) => {}
            }
        }
        if !stepped {
            break;
        }
    }
    Ok(acc)
}

fn pattern_indexes(pat: &Pattern) -> bool {
    !matches!(pat, Pattern::Var(_))
}

/// Taking a constant apart is an invalid path. The element named is the
/// first one jq tries: the last array slot, or the first object field.
fn destructure_near(pat: &Pattern, value: &Value, ctx: &Ctx) -> PathErr {
    let line = Cell::new(0);
    let body = Filter::Identity;
    let names = Vec::new();
    let tail = Tail {
        body: &body,
        orig_input: value,
        orig_input_gen: 0,
        names: &names,
    };
    match exec_pattern(
        pat,
        value,
        &[],
        &line,
        false,
        0,
        &[],
        Vec::new(),
        &tail,
        ctx,
    ) {
        Ok(_) => bare(near_access(&Value::Null, value)),
        Err(err) => err,
    }
}

fn run_match_pipe(
    regex: &Filter,
    flags: &Filter,
    testing: bool,
    right: &Filter,
    input: &Value,
    path: &[Step],
    ctx: &Ctx,
) -> Result<Vec<POut>, PathErr> {
    let mut out = Vec::new();
    let mut path_err: Option<PathErr> = None;
    let result = stream::visit_match_impl(regex, flags, testing, input, ctx.env, |value| {
        let cell = Cell::new(0);
        match run(right, value, path, &cell, 0, ctx) {
            Ok(more) => {
                out.extend(more);
                Ok(false)
            }
            Err(err) => {
                path_err = Some(err);
                Ok(true)
            }
        }
    });
    if let Some(err) = path_err {
        return Err(err);
    }
    match result {
        Ok(_) => Ok(out),
        Err(failure) => Err(path_from(out, failure)),
    }
}

/// `test` yields a boolean. That boolean is not a path.
fn run_match_value(
    regex: &Filter,
    flags: &Filter,
    testing: bool,
    input: &Value,
    path: &[Step],
    line: &Cell<u64>,
    ctx: &Ctx,
) -> Result<Vec<POut>, PathErr> {
    let mut out = Vec::new();
    let mut path_err: Option<PathErr> = None;
    let result = stream::visit_match_impl(regex, flags, testing, input, ctx.env, |value| {
        let made = constant(value.clone(), path, line.get(), ctx.root);
        match path_steps(ctx.root, made) {
            Ok(steps) => {
                out.push(POut {
                    value: value.clone(),
                    path: steps,
                    value_gen: line.get(),
                    line: line.get(),
                    carrier: true,
                    force_broken: false,
                });
                Ok(false)
            }
            Err(broken) => {
                path_err = Some(bare(format!(
                    "Invalid path expression with result {}",
                    clipped(&broken, 29)
                )));
                Ok(true)
            }
        }
    });
    if let Some(err) = path_err {
        return Err(err);
    }
    match result {
        Ok(_) => Ok(out),
        Err(failure) => Err(path_from(out, failure)),
    }
}

fn path_from(partial: Vec<POut>, failure: Failure) -> PathErr {
    PathErr {
        partial,
        message: failure.message,
        caught: failure.caught,
        stop: failure.stop,
    }
}

fn finish_outs(out: Vec<POut>, err: Option<Failure>) -> Result<Vec<POut>, PathErr> {
    match err {
        Some(failure) => Err(PathErr {
            partial: out,
            message: failure.message,
            caught: failure.caught,
            stop: failure.stop,
        }),
        None => Ok(out),
    }
}

fn run_try(
    body: &Filter,
    handler: Option<&Filter>,
    input: &Value,
    path: &[Step],
    line: &Cell<u64>,
    input_gen: u64,
    ctx: &Ctx,
) -> Result<Vec<POut>, PathErr> {
    let err = match run(body, input, path, line, input_gen, ctx) {
        Ok(outs) => return Ok(outs),
        Err(err) => err,
    };
    let Some(handler) = handler else {
        return Ok(err.partial);
    };
    let caught = match err.caught {
        Some(value) => value,
        None => Value::String(crate::value::Text::from(err.message)),
    };
    // The handler is not a carrier. A null or bool can still match the root.
    let cell = Cell::new(0);
    match run(handler, &caught, path, &cell, 0, ctx) {
        Ok(more) => {
            let mut partial = err.partial;
            partial.extend(more);
            Ok(partial)
        }
        Err(herr) => Err(attach(err.partial, herr)),
    }
}

fn near_access(elem: &Value, value: &Value) -> String {
    format!(
        "Invalid path expression near attempt to access element {} of {}",
        clipped(elem, 14),
        clipped(value, 29)
    )
}

/// `?` on `]` skips a failed index. It does not skip an invalid path.
fn skips_failed_index(message: &str) -> bool {
    !message.starts_with("Invalid path expression")
}

fn near_iter(value: &Value) -> String {
    format!(
        "Invalid path expression near attempt to iterate through {}",
        clipped(value, 29)
    )
}

fn shown_slice(start: &Value, end: &Value) -> Value {
    Value::object(vec![
        ("start".to_string(), render_bound(start)),
        ("end".to_string(), render_bound(end)),
    ])
}

fn run_field(
    key: &str,
    input: &Value,
    path: &[Step],
    line: &Cell<u64>,
    input_gen: u64,
    ctx: &Ctx,
) -> Result<Vec<POut>, PathErr> {
    if !carrier_now(line, input_gen) {
        let elem = Value::String(crate::value::Text::from(key.to_string()));
        return Err(bare(near_access(&elem, input)));
    }
    match input {
        Value::Null | Value::Object(_) => {
            let child = object_field(input, key);
            let mut full = path.to_vec();
            full.push(Step::Key(key.to_string()));
            Ok(vec![taken(child, full, line, ctx.next)])
        }
        other => Err(bare(format!(
            "Cannot index {} with string \"{key}\"",
            other.type_name()
        ))),
    }
}

fn run_fixed_index(
    index: &ArrayIndex,
    text: &str,
    input: &Value,
    path: &[Step],
    line: &Cell<u64>,
    input_gen: u64,
    ctx: &Ctx,
) -> Result<Vec<POut>, PathErr> {
    match index {
        ArrayIndex::At(_) => {
            let key = Value::Number(crate::value::Text::from(text.to_string()));
            index_here(&key, input, path, line, input_gen, ctx)
        }
        ArrayIndex::OutOfRange => {
            if !carrier_now(line, input_gen) {
                return Err(bare(near_access(&Value::Null, input)));
            }
            if matches!(input, Value::Null | Value::Array(_)) {
                Ok(vec![huge_out(path, text, line, ctx.next)])
            } else {
                Err(bare(format!(
                    "Cannot index {} with number",
                    input.type_name()
                )))
            }
        }
    }
}

fn run_slice(
    base: &Option<Box<Filter>>,
    start: &Option<Box<Filter>>,
    end: &Option<Box<Filter>>,
    optional: bool,
    input: &Value,
    path: &[Step],
    line: &Cell<u64>,
    input_gen: u64,
    ctx: &Ctx,
) -> Result<Vec<POut>, PathErr> {
    let identity = Filter::Identity;
    let base = base.as_deref().unwrap_or(&identity);
    let saved = line.get();
    let mut outs = Vec::new();
    let walked = walk_slice_pairs(
        start.as_deref(),
        end.as_deref(),
        input,
        ctx.env,
        |start_value, end_value| {
            line.set(saved);
            let base_run = run(base, input, path, line, input_gen, ctx);
            let (base_outs, base_err) = match base_run {
                Ok(outs) => (outs, None),
                Err(err) => {
                    let (partial, failure) = split_err(err);
                    (partial, Some(failure))
                }
            };
            for base_out in base_outs {
                if !base_out.carrier {
                    let elem = shown_slice(start_value, end_value);
                    return Err(fail(near_access(&elem, &base_out.value)));
                }
                match &base_out.value {
                    Value::Array(_) | Value::String(_) => {
                        if require_slice_index(start_value).is_err()
                            || require_slice_index(end_value).is_err()
                        {
                            if optional {
                                continue;
                            }
                            require_slice_index(start_value)?;
                            require_slice_index(end_value)?;
                        }
                    }
                    Value::Null => {}
                    other => {
                        if optional {
                            continue;
                        }
                        return Err(fail(format!(
                            "Cannot index {} with object",
                            other.type_name()
                        )));
                    }
                }
                let step = Step::Slice {
                    start: start_value.clone(),
                    end: end_value.clone(),
                };
                let child = child_at(&base_out.value, &step).map_err(fail)?;
                let mut full = base_out.path;
                full.push(step);
                outs.push(taken(child, full, line, ctx.next));
            }
            match base_err {
                Some(failure) => Err(failure),
                None => Ok(()),
            }
        },
    );
    match walked {
        Ok(()) => Ok(outs),
        Err(failure) => Err(PathErr {
            partial: outs,
            message: failure.message,
            caught: failure.caught,
            stop: failure.stop,
        }),
    }
}

fn run_iterate(
    input: &Value,
    path: &[Step],
    line: &Cell<u64>,
    input_gen: u64,
    ctx: &Ctx,
) -> Result<Vec<POut>, PathErr> {
    if !carrier_now(line, input_gen) {
        return Err(bare(near_iter(input)));
    }
    match input {
        Value::Array(items) => {
            let mut out = Vec::new();
            for (index, item) in items.iter().enumerate() {
                let mut full = path.to_vec();
                full.push(index_step(index as i64, &index.to_string()));
                out.push(taken(item.clone(), full, line, ctx.next));
            }
            Ok(out)
        }
        Value::Object(items) => {
            let mut out = Vec::new();
            for (key, item) in items {
                let mut full = path.to_vec();
                full.push(Step::Key(key.clone()));
                out.push(taken(item.clone(), full, line, ctx.next));
            }
            Ok(out)
        }
        other => Err(bare(format!(
            "Cannot iterate over {} ({})",
            other.type_name(),
            shown(other)
        ))),
    }
}

fn run_recurse(
    input: &Value,
    path: &[Step],
    line: &Cell<u64>,
    input_gen: u64,
    ctx: &Ctx,
) -> Result<Vec<POut>, PathErr> {
    if !carrier_now(line, input_gen) {
        return Ok(vec![constant(input.clone(), path, line.get(), ctx.root)]);
    }
    let mut out = vec![pass(input, path, line, input_gen, ctx.root)];
    match input {
        Value::Array(items) => {
            for (index, item) in items.iter().enumerate() {
                let gen = alloc(ctx.next);
                let cell = Cell::new(gen);
                let mut child_path = path.to_vec();
                child_path.push(index_step(index as i64, &index.to_string()));
                match run(&Filter::Recurse, item, &child_path, &cell, gen, ctx) {
                    Ok(more) => out.extend(more),
                    Err(err) => return Err(attach(out, err)),
                }
            }
        }
        Value::Object(items) => {
            for (key, item) in items {
                let gen = alloc(ctx.next);
                let cell = Cell::new(gen);
                let mut child_path = path.to_vec();
                child_path.push(Step::Key(key.clone()));
                match run(&Filter::Recurse, item, &child_path, &cell, gen, ctx) {
                    Ok(more) => out.extend(more),
                    Err(err) => return Err(attach(out, err)),
                }
            }
        }
        _ => {}
    }
    Ok(out)
}

/// `recurse(f)` and `recurse(f; cond)` as a path. The current value is
/// included, then each output of `f` is entered once per truthy `cond`
/// result. An output that was not reached by indexing is an invalid path,
/// and a later output of `f` is left for the next index so a following
/// error is not raised first.
fn run_recurse_down(
    step: &Filter,
    cond: Option<&Filter>,
    input: &Value,
    path: &[Step],
    line: &Cell<u64>,
    input_gen: u64,
    ctx: &Ctx,
) -> Result<Vec<POut>, PathErr> {
    if !carrier_now(line, input_gen) {
        return Ok(vec![constant(input.clone(), path, line.get(), ctx.root)]);
    }
    let mut out = vec![pass(input, path, line, input_gen, ctx.root)];
    let mut step_i = 0usize;
    loop {
        let child = match nth_path(step, input, path, line, input_gen, ctx, step_i) {
            Ok(Some(child)) => child,
            Ok(None) => break,
            Err(err) => return Err(attach(out, err)),
        };
        step_i += 1;
        if !child.carrier {
            out.push(child);
            break;
        }
        let mut cond_i = 0usize;
        loop {
            let go = match cond {
                Some(cond) => match truthy_at(cond, &child.value, ctx.env, cond_i) {
                    Ok(hit) => hit,
                    Err(err) => return Err(attach(out, err)),
                },
                None => cond_i == 0,
            };
            if !go {
                break;
            }
            cond_i += 1;
            let cell = Cell::new(child.value_gen);
            match run_recurse_down(
                step,
                cond,
                &child.value,
                &child.path,
                &cell,
                child.value_gen,
                ctx,
            ) {
                Ok(more) => out.extend(more),
                Err(err) => return Err(attach(out, err)),
            }
        }
    }
    Ok(out)
}

fn nth_path(
    step: &Filter,
    input: &Value,
    path: &[Step],
    line: &Cell<u64>,
    input_gen: u64,
    ctx: &Ctx,
    index: usize,
) -> Result<Option<POut>, PathErr> {
    let saved = line.get();
    let result = run_take(step, input, path, line, input_gen, ctx, index + 1);
    line.set(saved);
    match result {
        Ok(mut got) => {
            if got.len() <= index {
                Ok(None)
            } else {
                Ok(Some(got.swap_remove(index)))
            }
        }
        // Outputs before this index were already walked. Keeping them here
        // would report those paths twice.
        Err(err) => Err(PathErr {
            partial: Vec::new(),
            message: err.message,
            caught: err.caught,
            stop: err.stop,
        }),
    }
}

fn truthy_at(
    cond: &Filter,
    input: &Value,
    env: &[(String, Value)],
    index: usize,
) -> Result<bool, PathErr> {
    match stream::nth_truthy(cond, input, env, index) {
        Ok(hit) => Ok(hit),
        Err(failure) => Err(PathErr {
            partial: Vec::new(),
            message: failure.message,
            caught: failure.caught,
            stop: failure.stop,
        }),
    }
}

fn run_optional(
    inner: &Filter,
    input: &Value,
    path: &[Step],
    line: &Cell<u64>,
    input_gen: u64,
    ctx: &Ctx,
) -> Result<Vec<POut>, PathErr> {
    // `.[expr]?` keeps the inner flag, so a key, bound, or base error still
    // fails. `(expr)?` swallows the error and keeps outputs from before it.
    if matches!(
        inner,
        Filter::At { optional: true, .. } | Filter::Slice { optional: true, .. }
    ) {
        return run(inner, input, path, line, input_gen, ctx);
    }
    match run(inner, input, path, line, input_gen, ctx) {
        Ok(outs) => Ok(outs),
        Err(err) => Ok(err.partial),
    }
}

fn run_pipe(
    left: &Filter,
    right: &Filter,
    input: &Value,
    path: &[Step],
    line: &Cell<u64>,
    input_gen: u64,
    ctx: &Ctx,
) -> Result<Vec<POut>, PathErr> {
    if let Filter::MatchImpl {
        regex,
        flags,
        testing,
    } = left
    {
        return run_match_pipe(regex, flags, *testing, right, input, path, ctx);
    }
    let left_cell = Cell::new(line.get());
    let (lefts, left_err) = match run(left, input, path, &left_cell, input_gen, ctx) {
        Ok(outs) => (outs, None),
        Err(err) => {
            let (partial, failure) = split_err(err);
            (partial, Some(failure))
        }
    };
    let mut out = Vec::new();
    for left in lefts {
        // A non-carrier keeps the lineage number and still refuses indexes.
        let (gen, next_gen) = if left.carrier {
            (left.value_gen, left.value_gen)
        } else {
            (left.line, 0)
        };
        let cell = Cell::new(gen);
        match run(right, &left.value, &left.path, &cell, next_gen, ctx) {
            Ok(more) => out.extend(more),
            Err(err) => return Err(attach(out, err)),
        }
    }
    finish_outs(out, left_err)
}

fn run_comma(
    left: &Filter,
    right: &Filter,
    input: &Value,
    path: &[Step],
    line: &Cell<u64>,
    input_gen: u64,
    ctx: &Ctx,
) -> Result<Vec<POut>, PathErr> {
    let saved = line.get();
    let left_cell = Cell::new(saved);
    let lefts = run(left, input, path, &left_cell, input_gen, ctx)?;
    let right_cell = Cell::new(saved);
    match run(right, input, path, &right_cell, input_gen, ctx) {
        Ok(rights) => {
            let mut out = lefts;
            out.extend(rights);
            Ok(out)
        }
        Err(err) => Err(attach(lefts, err)),
    }
}

fn run_alt(
    left: &Filter,
    right: &Filter,
    input: &Value,
    path: &[Step],
    line: &Cell<u64>,
    input_gen: u64,
    ctx: &Ctx,
) -> Result<Vec<POut>, PathErr> {
    let saved_line = line.get();
    let saved_path = path.to_vec();
    match run(left, input, path, line, input_gen, ctx) {
        Err(err) => {
            let partial = err
                .partial
                .into_iter()
                .filter(|out| truthy(&out.value))
                .collect();
            Err(PathErr {
                partial,
                message: err.message,
                caught: err.caught,
                stop: err.stop,
            })
        }
        Ok(lefts) => {
            let defined: Vec<POut> = lefts.into_iter().filter(|out| truthy(&out.value)).collect();
            if defined.is_empty() {
                line.set(saved_line);
                run(right, input, &saved_path, line, input_gen, ctx)
            } else {
                Ok(defined)
            }
        }
    }
}

/// `and` yields false when the left is falsy. `or` yields true when it is truthy.
/// The other side indexes the input this operator started from, at the left's lineage.
fn run_bool(
    left: &Filter,
    right: &Filter,
    input: &Value,
    path: &[Step],
    line: &Cell<u64>,
    input_gen: u64,
    ctx: &Ctx,
    or: bool,
) -> Result<Vec<POut>, PathErr> {
    let (lefts, left_err) = match run(left, input, path, line, input_gen, ctx) {
        Ok(outs) => (outs, None),
        Err(err) => {
            let (partial, failure) = split_err(err);
            (partial, Some(failure))
        }
    };
    let mut out = Vec::new();
    for left in lefts {
        let stop = if or {
            truthy(&left.value)
        } else {
            !truthy(&left.value)
        };
        if stop {
            out.push(constant(Value::Bool(or), &left.path, left.line, ctx.root));
            continue;
        }
        let cell = Cell::new(left.line);
        match run(right, input, &left.path, &cell, input_gen, ctx) {
            Ok(rights) => {
                for right in rights {
                    out.push(wrap_bool(&right, ctx.root));
                }
            }
            Err(err) => {
                for right in err.partial {
                    out.push(wrap_bool(&right, ctx.root));
                }
                return Err(PathErr {
                    partial: out,
                    message: err.message,
                    caught: err.caught,
                    stop: err.stop,
                });
            }
        }
    }
    finish_outs(out, left_err)
}

fn wrap_bool(out: &POut, root: &Value) -> POut {
    constant(Value::Bool(truthy(&out.value)), &out.path, out.line, root)
}

fn run_if(
    cond: &Filter,
    yes: &Filter,
    no: &Filter,
    input: &Value,
    path: &[Step],
    line: &Cell<u64>,
    input_gen: u64,
    ctx: &Ctx,
) -> Result<Vec<POut>, PathErr> {
    let saved = line.get();
    let (conds, err) = parts(eval_in(cond, input, ctx.env));
    line.set(saved);
    let mut out = Vec::new();
    for cond_value in conds {
        line.set(saved);
        let branch = if truthy(&cond_value) { yes } else { no };
        match run(branch, input, path, line, input_gen, ctx) {
            Ok(more) => out.extend(more),
            Err(err) => return Err(attach(out, err)),
        }
    }
    finish_outs(out, err)
}

fn run_select(
    cond: &Filter,
    input: &Value,
    path: &[Step],
    line: &Cell<u64>,
    input_gen: u64,
    ctx: &Ctx,
) -> Result<Vec<POut>, PathErr> {
    let saved = line.get();
    let (flags, err) = parts(eval_in(cond, input, ctx.env));
    line.set(saved);
    let mut out = Vec::new();
    for flag in flags {
        if truthy(&flag) {
            out.push(pass(input, path, line, input_gen, ctx.root));
        }
    }
    finish_outs(out, err)
}

fn run_bind(
    expr: &Filter,
    pattern: &Pattern,
    alts: &[Pattern],
    body: &Filter,
    input: &Value,
    path: &[Step],
    line: &Cell<u64>,
    input_gen: u64,
    ctx: &Ctx,
) -> Result<Vec<POut>, PathErr> {
    let mut names = Vec::new();
    super::parse::bind_names(pattern, alts, &mut names);
    let tail = Tail {
        body,
        orig_input: input,
        orig_input_gen: input_gen,
        names: &names,
    };
    let mut pats = Vec::with_capacity(1 + alts.len());
    pats.push(pattern);
    pats.extend(alts);
    if matches!(expr, Filter::Identity) {
        let carrier = carrier_now(line, input_gen);
        return bind_patterns(&pats, input, path, line, carrier, input_gen, &tail, ctx);
    }
    let saved = line.get();
    let saved_path = path.to_vec();
    let (values, err) = parts(eval_in(expr, input, ctx.env));
    line.set(saved);
    let mut out = Vec::new();
    for value in values {
        line.set(saved);
        match bind_patterns(&pats, &value, &saved_path, line, false, 0, &tail, ctx) {
            Ok(more) => out.extend(more),
            Err(err) => return Err(attach(out, err)),
        }
    }
    finish_outs(out, err)
}

/// Names the successful pattern did not bind are null.
fn null_names(frame: &[(String, Value)], names: &[String]) -> Vec<(String, Value)> {
    let mut out = frame.to_vec();
    for name in names {
        if !out.iter().any(|(bound, _)| bound == name) {
            out.push((name.clone(), Value::Null));
        }
    }
    out
}

/// Each pattern in order. A failure that is not `halt` tries the next pattern.
fn bind_patterns(
    pats: &[&Pattern],
    value: &Value,
    path: &[Step],
    line: &Cell<u64>,
    carrier: bool,
    input_gen: u64,
    tail: &Tail,
    ctx: &Ctx,
) -> Result<Vec<POut>, PathErr> {
    // Indexing inside a pattern moves the lineage. Each `?//` pattern
    // starts again from the lineage this binding was reached at, so a
    // later `$name` is still a path back to the input.
    let saved = line.get();
    let mut out = Vec::new();
    for (index, pat) in pats.iter().enumerate() {
        line.set(saved);
        let last = index + 1 == pats.len();
        match exec_pattern(
            pat,
            value,
            path,
            line,
            carrier,
            input_gen,
            &[],
            Vec::new(),
            tail,
            ctx,
        ) {
            Ok(more) => {
                out.extend(more);
                return Ok(out);
            }
            Err(err) if err.stop.is_some() || last => return Err(attach(out, err)),
            Err(err) => out.extend(err.partial),
        }
    }
    Ok(out)
}

fn run_label(
    slot: u32,
    body: &Filter,
    input: &Value,
    path: &[Step],
    line: &Cell<u64>,
    input_gen: u64,
    ctx: &Ctx,
) -> Result<Vec<POut>, PathErr> {
    let (runtime, guard) = super::eval::LabelGuard::enter(slot);
    let result = run(body, input, path, line, input_gen, ctx);
    drop(guard);
    match result {
        Ok(outs) => Ok(outs),
        Err(err) if path_label_hit(&err, runtime) => Ok(err.partial),
        Err(err) => Err(err),
    }
}

fn path_break(slot: Option<u32>) -> PathErr {
    let Some(runtime) = slot.and_then(super::eval::lookup_label) else {
        return bare("break".to_string());
    };
    let value = super::eval::label_object(runtime);
    PathErr {
        partial: Vec::new(),
        message: super::eval::jq_text(&value),
        caught: Some(value),
        stop: None,
    }
}

fn path_label_hit(err: &PathErr, runtime: u32) -> bool {
    if err.stop.is_some() {
        return false;
    }
    match &err.caught {
        Some(value) => super::eval::is_label_hit(
            &Failure {
                partial: Vec::new(),
                message: String::new(),
                caught: Some(value.clone()),
                stop: None,
            },
            runtime,
        ),
        None => false,
    }
}

fn run_at(
    base: &Filter,
    key: &Filter,
    optional: bool,
    input: &Value,
    path: &[Step],
    line: &Cell<u64>,
    input_gen: u64,
    ctx: &Ctx,
) -> Result<Vec<POut>, PathErr> {
    let saved = line.get();
    let (keys, key_err) = parts(eval_in(key, input, ctx.env));
    line.set(saved);
    let mut out = Vec::new();
    for key_value in &keys {
        line.set(saved);
        let bases = match run(base, input, path, line, input_gen, ctx) {
            Ok(bases) => bases,
            Err(err) => return Err(attach(out, err)),
        };
        for base_out in bases {
            match index_out(&base_out, key_value, ctx) {
                Ok(hit) => out.push(hit),
                Err(message) if optional && skips_failed_index(&message) => {
                    let _ = message;
                }
                Err(message) => {
                    let (message, caught, stop) = match key_err.clone() {
                        Some(failure) => (failure.message, failure.caught, failure.stop),
                        None => (message, None, None),
                    };
                    return Err(PathErr {
                        partial: out,
                        message,
                        caught,
                        stop,
                    });
                }
            }
        }
    }
    finish_outs(out, key_err)
}

fn index_here(
    key: &Value,
    input: &Value,
    path: &[Step],
    line: &Cell<u64>,
    input_gen: u64,
    ctx: &Ctx,
) -> Result<Vec<POut>, PathErr> {
    if !carrier_now(line, input_gen) {
        return Err(bare(near_access(key, input)));
    }
    match real_step(input, key).map_err(bare)? {
        Real::Huge(text) => Ok(vec![huge_out(path, &text, line, ctx.next)]),
        Real::Done(step, child) => {
            let mut full = path.to_vec();
            full.push(step);
            Ok(vec![taken(child, full, line, ctx.next)])
        }
    }
}

fn index_out(base: &POut, key: &Value, ctx: &Ctx) -> Result<POut, String> {
    if !base.carrier {
        return Err(near_access(key, &base.value));
    }
    match real_step(&base.value, key)? {
        Real::Huge(text) => Ok(huge_out(&base.path, &text, &Cell::new(base.line), ctx.next)),
        Real::Done(step, child) => {
            let gen = alloc(ctx.next);
            let mut path = base.path.clone();
            path.push(step);
            Ok(POut {
                value: child,
                path,
                value_gen: gen,
                line: gen,
                carrier: true,
                force_broken: false,
            })
        }
    }
}

fn huge_out(path: &[Step], text: &str, line: &Cell<u64>, next: &Cell<u64>) -> POut {
    let mut full = path.to_vec();
    full.push(Step::Huge(text.to_string()));
    // A negative value that does not fit is still an index, so a later
    // `.a` or `[0]` extends it. Setting it is out of bounds. A positive
    // one stays a broken path.
    if negative_huge(text) {
        let gen = alloc(next);
        line.set(gen);
        return POut {
            value: Value::Null,
            path: full,
            value_gen: gen,
            line: gen,
            carrier: true,
            force_broken: false,
        };
    }
    POut {
        value: Value::Null,
        path: full,
        value_gen: 0,
        line: line.get(),
        carrier: false,
        force_broken: true,
    }
}

/// Negative, and not a number `array_index` can store. `nan` is not negative.
fn negative_huge(text: &str) -> bool {
    match crate::number::real(text) {
        crate::number::Real::Num(number) => number.is_sign_negative(),
        crate::number::Real::Nan => false,
    }
}

fn huge_index_error(text: &str) -> String {
    match crate::number::real(text) {
        crate::number::Real::Nan => "Cannot set array element at NaN index".to_string(),
        crate::number::Real::Num(number) if number.is_sign_negative() => {
            "Out of bounds negative array index".to_string()
        }
        _ => "Array index too large".to_string(),
    }
}

fn index_step(at: i64, text: &str) -> Step {
    Step::Index {
        at,
        text: canonical_number(text),
    }
}

fn real_step(container: &Value, key: &Value) -> Result<Real, String> {
    match key {
        Value::String(name) => match container {
            Value::Null | Value::Object(_) => Ok(Real::Done(
                Step::Key(name.to_string()),
                object_field(container, name),
            )),
            other => Err(format!(
                "Cannot index {} with string \"{name}\"",
                other.type_name()
            )),
        },
        Value::Number(text) => {
            let shown = canonical_number(text);
            let Some(index) = array_index(&shown) else {
                return if matches!(container, Value::Null | Value::Array(_)) {
                    Ok(Real::Huge(shown))
                } else {
                    Err(format!(
                        "Cannot index {} with number",
                        container.type_name()
                    ))
                };
            };
            let step = index_step(index, &shown);
            match container {
                Value::Null => Ok(Real::Done(step, Value::Null)),
                Value::Array(items) => Ok(Real::Done(step, nth_value(items, index))),
                other => Err(format!("Cannot index {} with number", other.type_name())),
            }
        }
        Value::Object(fields) => match container {
            Value::Null | Value::Array(_) | Value::String(_) => {
                let (start, end) = slice_from_object(fields)?;
                let probe = Step::Slice {
                    start: as_bound(start),
                    end: as_bound(end),
                };
                let child = child_at(container, &probe)?;
                // `path` prints this object as it was written, extra keys
                // included. A colon slice is still start then end.
                Ok(Real::Done(Step::Raw(key.clone()), child))
            }
            other => Err(format!("Cannot index {} with object", other.type_name())),
        },
        Value::Array(_) if matches!(container, Value::Array(_)) => {
            Err("Cannot update field at array index of array".to_string())
        }
        other => Err(format!(
            "Cannot index {} with {}",
            container.type_name(),
            other.type_name()
        )),
    }
}

fn slice_from_object(
    fields: &[(String, Value)],
) -> Result<(Option<String>, Option<String>), String> {
    super::object_slice_bounds(fields).map_err(|failure| failure.message)
}

fn bound_text(value: &Value) -> Result<Option<String>, String> {
    match value {
        Value::Null => Ok(None),
        Value::Number(text) => Ok(Some(text.strip_prefix('\u{1}').unwrap_or(text).to_string())),
        _ => Err("Array/string slice indices must be integers".to_string()),
    }
}

fn as_bound(bound: Option<String>) -> Value {
    match bound {
        Some(text) => Value::Number(crate::value::Text::from(text)),
        None => Value::Null,
    }
}

fn object_field(value: &Value, key: &str) -> Value {
    match value {
        Value::Object(items) => items
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, item)| item.clone())
            .unwrap_or(Value::Null),
        _ => Value::Null,
    }
}

fn child_at(value: &Value, step: &Step) -> Result<Value, String> {
    value_at(value, std::slice::from_ref(step))
}

/// A modify update inside `path` is the first element of `[update, []]`.
fn run_modify_path(
    filter: &Filter,
    input: &Value,
    path: &[Step],
    line: &Cell<u64>,
    ctx: &Ctx,
) -> Result<Vec<POut>, PathErr> {
    let lineage = line.get();
    let values = match eval_in(filter, input, ctx.env) {
        Ok(values) => values,
        Err(failure) => {
            let partial = failure
                .partial
                .into_iter()
                .map(|value| constant(value, path, lineage, ctx.root))
                .collect();
            return Err(PathErr {
                partial,
                message: failure.message,
                caught: failure.caught,
                stop: failure.stop,
            });
        }
    };
    if values.is_empty() {
        return Ok(Vec::new());
    }
    let mut items = values;
    items.push(Value::array(Vec::new()));
    let shown = Value::array(items);
    let zero = Value::Number(crate::value::Text::from("0"));
    Err(bare(near_access(&zero, &shown)))
}

fn run_fallback(
    filter: &Filter,
    input: &Value,
    path: &[Step],
    line: &Cell<u64>,
    ctx: &Ctx,
) -> Result<Vec<POut>, PathErr> {
    let lineage = line.get();
    match eval_in(filter, input, ctx.env) {
        Ok(values) => Ok(values
            .into_iter()
            .map(|value| constant(value, path, lineage, ctx.root))
            .collect()),
        Err(failure) => {
            let partial = failure
                .partial
                .into_iter()
                .map(|value| constant(value, path, lineage, ctx.root))
                .collect();
            Err(PathErr {
                partial,
                message: failure.message,
                caught: failure.caught,
                stop: failure.stop,
            })
        }
    }
}

fn exec_pattern(
    pattern: &Pattern,
    value: &Value,
    path: &[Step],
    line: &Cell<u64>,
    carrier: bool,
    input_gen: u64,
    binds: &[PBind],
    jobs: Vec<Job>,
    tail: &Tail,
    ctx: &Ctx,
) -> Result<Vec<POut>, PathErr> {
    match pattern {
        Pattern::Var(name) => {
            let mut next = binds.to_vec();
            next.push(capture(name, value, path, line, carrier, input_gen));
            resume(jobs, &next, path, line, tail, ctx)
        }
        Pattern::Array(pats) => exec_array_from(
            pats,
            pats.len(),
            value,
            path,
            line,
            carrier,
            input_gen,
            binds,
            jobs,
            tail,
            ctx,
        ),
        Pattern::Object(fields) => exec_obj_from(
            fields, 0, value, path, line, carrier, input_gen, binds, jobs, tail, ctx,
        ),
    }
}

fn resume(
    mut jobs: Vec<Job>,
    binds: &[PBind],
    path: &[Step],
    line: &Cell<u64>,
    tail: &Tail,
    ctx: &Ctx,
) -> Result<Vec<POut>, PathErr> {
    let Some(job) = jobs.first().cloned() else {
        return run_body(binds, path, line.get(), tail, ctx);
    };
    jobs.remove(0);
    match job {
        Job::Arr {
            pats,
            value,
            carrier,
            input_gen,
            index,
        } => exec_array_from(
            &pats, index, &value, path, line, carrier, input_gen, binds, jobs, tail, ctx,
        ),
        Job::Obj {
            fields,
            value,
            carrier,
            input_gen,
            index,
        } => exec_obj_from(
            &fields, index, &value, path, line, carrier, input_gen, binds, jobs, tail, ctx,
        ),
    }
}

fn exec_array_from(
    pats: &[Pattern],
    index: usize,
    value: &Value,
    path: &[Step],
    line: &Cell<u64>,
    carrier: bool,
    input_gen: u64,
    binds: &[PBind],
    mut jobs: Vec<Job>,
    tail: &Tail,
    ctx: &Ctx,
) -> Result<Vec<POut>, PathErr> {
    if index == 0 {
        return resume(jobs, binds, path, line, tail, ctx);
    }
    let key = Value::Number(crate::value::Text::from((index - 1).to_string()));
    let hit = pat_index(value, &key, path, line, carrier, input_gen, ctx)?;
    let sub = index - 1;
    jobs.insert(
        0,
        Job::Arr {
            pats: pats.to_vec(),
            value: value.clone(),
            carrier,
            input_gen,
            index: sub,
        },
    );
    exec_pattern(
        &pats[sub],
        &hit.value,
        &hit.path,
        line,
        true,
        hit.input_gen,
        binds,
        jobs,
        tail,
        ctx,
    )
}

fn exec_obj_from(
    fields: &[PatField],
    index: usize,
    value: &Value,
    path: &[Step],
    line: &Cell<u64>,
    carrier: bool,
    input_gen: u64,
    binds: &[PBind],
    jobs: Vec<Job>,
    tail: &Tail,
    ctx: &Ctx,
) -> Result<Vec<POut>, PathErr> {
    let Some(field) = fields.get(index).cloned() else {
        return resume(jobs, binds, path, line, tail, ctx);
    };
    let saved_gen = line.get();
    let saved_path = path.to_vec();
    let (keys, key_err) = parts(eval_in(&field.key, value, ctx.env));
    line.set(saved_gen);
    let mut out = Vec::new();
    for key in &keys {
        line.set(saved_gen);
        let hit = match pat_index(value, key, &saved_path, line, carrier, input_gen, ctx) {
            Ok(hit) => hit,
            Err(err) => return Err(attach(out, err)),
        };
        let mut next_binds = binds.to_vec();
        if let Some(name) = &field.also {
            next_binds.push(capture(
                name,
                &hit.value,
                &hit.path,
                line,
                true,
                hit.input_gen,
            ));
        }
        let mut next_jobs = jobs.clone();
        next_jobs.insert(
            0,
            Job::Obj {
                fields: fields.to_vec(),
                value: value.clone(),
                carrier,
                input_gen,
                index: index + 1,
            },
        );
        match exec_pattern(
            &field.pattern,
            &hit.value,
            &hit.path,
            line,
            true,
            hit.input_gen,
            &next_binds,
            next_jobs,
            tail,
            ctx,
        ) {
            Ok(more) => out.extend(more),
            Err(err) => return Err(attach(out, err)),
        }
    }
    finish_outs(out, key_err)
}

fn pat_index(
    value: &Value,
    key: &Value,
    path: &[Step],
    line: &Cell<u64>,
    carrier: bool,
    input_gen: u64,
    ctx: &Ctx,
) -> Result<Hit, PathErr> {
    if !(carrier && carrier_now(line, input_gen)) {
        return Err(bare(near_access(key, value)));
    }
    match real_step(value, key).map_err(bare)? {
        Real::Huge(_) => Err(bare(format!(
            "Cannot index {} with number",
            value.type_name()
        ))),
        Real::Done(step, child) => {
            let gen = alloc(ctx.next);
            line.set(gen);
            let mut full = path.to_vec();
            full.push(step);
            Ok(Hit {
                value: child,
                path: full,
                input_gen: gen,
            })
        }
    }
}

fn capture(
    name: &str,
    value: &Value,
    path: &[Step],
    line: &Cell<u64>,
    carrier: bool,
    input_gen: u64,
) -> PBind {
    let live = carrier && carrier_now(line, input_gen);
    PBind {
        name: name.to_string(),
        value: value.clone(),
        path: path.to_vec(),
        value_gen: if live { line.get() } else { 0 },
        line: line.get(),
        carrier: live,
    }
}

fn run_body(
    binds: &[PBind],
    path: &[Step],
    line_gen: u64,
    tail: &Tail,
    ctx: &Ctx,
) -> Result<Vec<POut>, PathErr> {
    let mut frame: Vec<(String, Value)> = binds
        .iter()
        .map(|bind| (bind.name.clone(), bind.value.clone()))
        .collect();
    for name in tail.names {
        if !frame.iter().any(|(bound, _)| bound == name) {
            frame.push((name.clone(), Value::Null));
        }
    }
    let child_env = child_scope(&frame, ctx.env);
    let mut merged = binds.to_vec();
    merged.extend_from_slice(ctx.pbinds);
    let cell = Cell::new(line_gen);
    let child = Ctx {
        root: ctx.root,
        next: ctx.next,
        env: &child_env,
        pbinds: &merged,
    };
    run(
        tail.body,
        tail.orig_input,
        path,
        &cell,
        tail.orig_input_gen,
        &child,
    )
}

fn emit_bind(bind: &PBind, root: &Value) -> POut {
    let mut out = POut {
        value: bind.value.clone(),
        path: bind.path.clone(),
        value_gen: bind.value_gen,
        line: bind.line,
        carrier: bind.carrier,
        force_broken: false,
    };
    if !out.carrier {
        upgrade(&mut out, root);
    }
    out
}

fn value_at(value: &Value, path: &[Step]) -> Result<Value, String> {
    if path.is_empty() {
        return Ok(value.clone());
    }
    let child = match (&path[0], value) {
        (Step::Raw(key), _) => {
            let got = get_one(value, key)?;
            return if path.len() == 1 {
                Ok(got)
            } else {
                value_at(&got, &path[1..])
            };
        }
        (Step::Key(_), Value::Null) => Value::Null,
        (Step::Key(key), Value::Object(items)) => items
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, item)| item.clone())
            .unwrap_or(Value::Null),
        (Step::Key(key), other) => {
            return Err(format!(
                "Cannot index {} with string \"{key}\"",
                other.type_name()
            ));
        }
        (Step::Index { .. } | Step::Huge(_), Value::Null) => Value::Null,
        (Step::Index { at: index, .. }, Value::Array(items)) => nth_value(items, *index),
        (Step::Huge(_), Value::Array(_)) => Value::Null,
        (Step::Index { .. } | Step::Huge(_), other) => {
            return Err(format!("Cannot index {} with number", other.type_name()));
        }
        (Step::Slice { start, end }, _) => {
            let one = if matches!(value, Value::Null) {
                Value::Null
            } else {
                let start = bound_text(start)?;
                let end = bound_text(end)?;
                let sliced = slice_value(value, start.as_deref(), end.as_deref())
                    .map_err(|failure| failure.message)?;
                sliced.into_iter().next().unwrap_or(Value::Null)
            };
            return if path.len() == 1 {
                Ok(one)
            } else {
                value_at(&one, &path[1..])
            };
        }
    };
    value_at(&child, &path[1..])
}

fn nth_value(items: &[Value], index: i64) -> Value {
    let Some(position) = resolve_index(index, items.len()) else {
        return Value::Null;
    };
    items.get(position).cloned().unwrap_or(Value::Null)
}

fn resolve_index(index: i64, len: usize) -> Option<usize> {
    if index < 0 {
        let position = len as i64 + index;
        if position < 0 {
            None
        } else {
            Some(position as usize)
        }
    } else {
        Some(index as usize)
    }
}

fn write_path(edit: &mut Edit, path: &[Step], write: &Write) -> Result<(), String> {
    if path.is_empty() {
        match write {
            Write::Put(value) => *edit = Edit::Pure(value.clone()),
            Write::Remove => *edit = Edit::Pure(Value::Null),
        }
        return Ok(());
    }
    match &path[0] {
        Step::Key(key) => write_key(edit, key, &path[1..], write),
        Step::Index { at, .. } => write_index(edit, *at, &path[1..], write),
        Step::Huge(text) => {
            if negative_huge(text) && matches!(write, Write::Remove) {
                Ok(())
            } else {
                Err(huge_index_error(text))
            }
        }
        Step::Raw(key) => write_raw(edit, key, &path[1..], write),
        Step::Slice { start, end } => write_slice(edit, start, end, &path[1..], write),
    }
}

fn write_raw(edit: &mut Edit, key: &Value, rest: &[Step], write: &Write) -> Result<(), String> {
    match key {
        Value::Array(_) => {
            let current = freeze(edit);
            if matches!(current, Value::Array(_)) {
                Err("Cannot update field at array index of array".to_string())
            } else {
                Err(format!("Cannot index {} with array", current.type_name()))
            }
        }
        Value::Object(fields) => {
            let current = freeze(edit);
            if !matches!(current, Value::Null | Value::Array(_) | Value::String(_)) {
                return Err(format!("Cannot index {} with object", current.type_name()));
            }
            let (start, end) = slice_from_object(fields)?;
            let start = as_bound(start);
            let end = as_bound(end);
            write_slice(edit, &start, &end, rest, write)
        }
        _ => {
            let current = freeze(edit);
            match real_step(&current, key)? {
                Real::Huge(text) => {
                    let mut steps = vec![Step::Huge(text)];
                    steps.extend_from_slice(rest);
                    write_path(edit, &steps, write)
                }
                Real::Done(step, _) => {
                    let mut steps = vec![step];
                    steps.extend_from_slice(rest);
                    write_path(edit, &steps, write)
                }
            }
        }
    }
}

fn write_key(edit: &mut Edit, key: &str, rest: &[Step], write: &Write) -> Result<(), String> {
    // Deleting through null does not create the object.
    if matches!(write, Write::Remove) && matches!(edit, Edit::Pure(Value::Null)) {
        return Ok(());
    }
    let fields = force_object(edit, key)?;
    if rest.is_empty() {
        match write {
            Write::Remove => {
                fields.retain(|(name, _)| name != key);
            }
            Write::Put(value) => put_edit(fields, key.to_string(), Edit::Pure(value.clone())),
        }
        return Ok(());
    }
    let child = fields
        .iter_mut()
        .find(|(name, _)| name == key)
        .map(|(_, child)| child);
    match (child, write) {
        (None, Write::Remove) => Ok(()),
        (None, Write::Put(_)) => {
            fields.push((key.to_string(), Edit::Pure(Value::Null)));
            let child = &mut fields.last_mut().expect("just pushed").1;
            write_path(child, rest, write)
        }
        (Some(child), _) => write_path(child, rest, write),
    }
}

fn force_object<'a>(edit: &'a mut Edit, key: &str) -> Result<&'a mut Vec<(String, Edit)>, String> {
    match edit {
        Edit::Obj(_) => {}
        Edit::Pure(Value::Null) => *edit = Edit::Obj(Vec::new()),
        Edit::Pure(Value::Object(items)) => {
            let taken = std::mem::replace(items, crate::value::Share::new(Vec::new()));
            let fields = taken
                .unwrap_or_clone()
                .into_iter()
                .map(|(name, value)| (name, Edit::Pure(value)))
                .collect();
            *edit = Edit::Obj(fields);
        }
        Edit::Pure(other) => {
            return Err(format!(
                "Cannot index {} with string \"{key}\"",
                other.type_name()
            ));
        }
        Edit::Arr(_) => {
            return Err("Cannot index array with string \"".to_string() + key + "\"");
        }
    }
    match edit {
        Edit::Obj(fields) => Ok(fields),
        _ => unreachable!("object"),
    }
}

fn put_edit(fields: &mut Vec<(String, Edit)>, key: String, value: Edit) {
    if let Some((_, slot)) = fields.iter_mut().find(|(name, _)| name == &key) {
        *slot = value;
    } else {
        fields.push((key, value));
    }
}

fn write_index(edit: &mut Edit, index: i64, rest: &[Step], write: &Write) -> Result<(), String> {
    if rest.is_empty() && matches!(write, Write::Remove) && matches!(edit, Edit::Pure(Value::Null))
    {
        return Ok(());
    }
    if let Edit::Pure(Value::String(_)) = edit {
        return Err("Cannot index string with number".to_string());
    }
    let arr = force_array(edit)?;
    let len = arr.slots.len();
    let Some(position) = resolve_index(index, len) else {
        return if matches!(write, Write::Remove) {
            Ok(())
        } else {
            Err("Out of bounds negative array index".to_string())
        };
    };
    if position >= arr.slots.len() {
        if matches!(write, Write::Remove) {
            return Ok(());
        }
        while arr.slots.len() <= position {
            arr.slots.push(live(Value::Null));
        }
    }
    if rest.is_empty() {
        match write {
            Write::Remove => arr.slots[position].body = Body::Gone,
            Write::Put(value) => arr.slots[position].body = Body::Live(Edit::Pure(value.clone())),
        }
        return Ok(());
    }
    if matches!(arr.slots[position].body, Body::Gone) {
        if matches!(write, Write::Remove) {
            return Ok(());
        }
        arr.slots[position].body = Body::Live(Edit::Pure(Value::Null));
    }
    let Body::Live(child) = &mut arr.slots[position].body else {
        return Ok(());
    };
    write_path(child, rest, write)
}

fn write_slice(
    edit: &mut Edit,
    start: &Value,
    end: &Value,
    rest: &[Step],
    write: &Write,
) -> Result<(), String> {
    if let Edit::Pure(Value::String(_)) = edit {
        bound_text(start)?;
        bound_text(end)?;
        return Err(match write {
            Write::Remove => "Cannot delete fields from string".to_string(),
            Write::Put(_) => "Cannot update string slices".to_string(),
        });
    }
    if matches!(write, Write::Remove) && matches!(edit, Edit::Pure(Value::Null)) {
        return Ok(());
    }
    if matches!(edit, Edit::Pure(Value::Object(_)) | Edit::Obj(_)) {
        return Err("Cannot index object with object".to_string());
    }
    if let Edit::Pure(other) = edit {
        if !matches!(other, Value::Null | Value::Array(_) | Value::String(_)) {
            return Err(format!("Cannot index {} with object", other.type_name()));
        }
    }
    let start_text = bound_text(start)?;
    let end_text = bound_text(end)?;
    if !rest.is_empty() {
        let current = read_edit(
            edit,
            &[Step::Slice {
                start: start.clone(),
                end: end.clone(),
            }],
        )?;
        let mut temp = Edit::Pure(current);
        write_path(&mut temp, rest, write)?;
        let value = freeze(&temp);
        return write_slice(edit, start, end, &[], &Write::Put(value));
    }
    match write {
        Write::Remove => {
            // Deletion keeps the original slots, so a later index is not moved
            // by an earlier removal.
            let arr = force_array(edit)?;
            let (from, to) =
                slice_range(start_text.as_deref(), end_text.as_deref(), arr.slots.len());
            if from >= to {
                return Ok(());
            }
            arr.slots[from].body = Body::Gone;
            for slot in &mut arr.slots[from + 1..to] {
                slot.before.clear();
                slot.body = Body::Gone;
            }
            Ok(())
        }
        Write::Put(value) => {
            // Assignment sees the array as it is now, after earlier slices.
            let Value::Array(items) = value else {
                return Err("A slice of an array can only be assigned another array".to_string());
            };
            let cur = match freeze(edit).into_array() {
                Ok(items) => items,
                Err(Value::Null) => Vec::new(),
                Err(other) => {
                    return Err(format!("Cannot index {} with object", other.type_name()));
                }
            };
            let (from, to) = slice_range(start_text.as_deref(), end_text.as_deref(), cur.len());
            let mut new = Vec::with_capacity(from + items.len() + cur.len().saturating_sub(to));
            new.extend_from_slice(&cur[..from]);
            new.extend(items.iter().cloned());
            new.extend_from_slice(&cur[to..]);
            *edit = Edit::Pure(Value::array(new));
            Ok(())
        }
    }
}

fn force_array(edit: &mut Edit) -> Result<&mut Arr, String> {
    match edit {
        Edit::Arr(_) => {}
        Edit::Pure(Value::Null) => {
            *edit = Edit::Arr(Arr {
                slots: Vec::new(),
                tail: Vec::new(),
            });
        }
        Edit::Pure(Value::Array(items)) => {
            let taken = std::mem::replace(items, crate::value::Share::new(Vec::new()));
            let slots = taken.unwrap_or_clone().into_iter().map(live).collect();
            *edit = Edit::Arr(Arr {
                slots,
                tail: Vec::new(),
            });
        }
        Edit::Pure(other) => {
            return Err(format!("Cannot index {} with number", other.type_name()));
        }
        Edit::Obj(_) => return Err("Cannot index object with number".to_string()),
    }
    match edit {
        Edit::Arr(arr) => Ok(arr),
        _ => unreachable!("array"),
    }
}

fn live(value: Value) -> Elem {
    Elem {
        before: Vec::new(),
        body: Body::Live(Edit::Pure(value)),
    }
}

fn read_edit(edit: &Edit, path: &[Step]) -> Result<Value, String> {
    if path.is_empty() {
        return Ok(freeze(edit));
    }
    match (edit, &path[0]) {
        (_, Step::Raw(key)) => {
            let current = freeze(edit);
            let got = get_one(&current, key)?;
            if path.len() == 1 {
                Ok(got)
            } else {
                value_at(&got, &path[1..])
            }
        }
        (Edit::Pure(value), _) => value_at(value, path),
        (Edit::Obj(fields), Step::Key(key)) => match fields.iter().find(|(name, _)| name == key) {
            Some((_, child)) => read_edit(child, &path[1..]),
            None => read_edit(&Edit::Pure(Value::Null), &path[1..]),
        },
        (Edit::Obj(_), Step::Index { .. } | Step::Huge(_)) => {
            Err("Cannot index object with number".to_string())
        }
        (Edit::Obj(_), Step::Slice { .. }) => Err("Cannot index object with object".to_string()),
        (Edit::Arr(_), Step::Huge(text)) => {
            if negative_huge(text) {
                read_edit(&Edit::Pure(Value::Null), &path[1..])
            } else {
                Err("Array index too large".to_string())
            }
        }
        (Edit::Arr(arr), Step::Index { at: index, .. }) => {
            let Some(position) = resolve_index(*index, arr.slots.len()) else {
                return Err("Out of bounds negative array index".to_string());
            };
            if position >= arr.slots.len() {
                return read_edit(&Edit::Pure(Value::Null), &path[1..]);
            }
            match &arr.slots[position].body {
                Body::Live(child) => read_edit(child, &path[1..]),
                Body::Gone => read_edit(&Edit::Pure(Value::Null), &path[1..]),
            }
        }
        (Edit::Arr(arr), Step::Slice { start, end }) => {
            let start = bound_text(start)?;
            let end = bound_text(end)?;
            let (from, to) = slice_range(start.as_deref(), end.as_deref(), arr.slots.len());
            let value = Value::array(slice_slots(arr, from, to));
            if path.len() == 1 {
                Ok(value)
            } else {
                read_edit(&Edit::Pure(value), &path[1..])
            }
        }
        (Edit::Arr(_), Step::Key(key)) => Err(format!("Cannot index array with string \"{key}\"")),
    }
}

fn slice_slots(arr: &Arr, from: usize, to: usize) -> Vec<Value> {
    let mut items = Vec::new();
    for index in from..to {
        let slot = &arr.slots[index];
        if index > from {
            items.extend(slot.before.iter().map(freeze));
        }
        items.extend(body_values(&slot.body));
    }
    items
}

fn body_values(body: &Body) -> Vec<Value> {
    match body {
        Body::Live(edit) => vec![freeze(edit)],
        Body::Gone => Vec::new(),
    }
}

fn freeze(edit: &Edit) -> Value {
    match edit {
        Edit::Pure(value) => value.clone(),
        Edit::Obj(fields) => Value::object(
            fields
                .iter()
                .map(|(key, value)| (key.clone(), freeze(value)))
                .collect(),
        ),
        Edit::Arr(arr) => {
            let mut items = Vec::new();
            for slot in &arr.slots {
                items.extend(slot.before.iter().map(freeze));
                items.extend(body_values(&slot.body));
            }
            items.extend(arr.tail.iter().map(freeze));
            Value::array(items)
        }
    }
}

pub(super) fn eval_path_of(
    filter: &Filter,
    input: &Value,
    env: &[(String, Value)],
) -> Result<Vec<Value>, Failure> {
    let walked = walk_paths(filter, input, env);
    let values = walked
        .steps
        .into_iter()
        .map(|steps| render_steps(&steps))
        .collect();
    match walked.failure {
        Some(failure) => Err(with_partial(values, failure)),
        None => Ok(values),
    }
}

/// `paths` is `path(..)` without the empty path. `paths(f)` keeps each path
/// once for every truthy output of `f` on the value there. `f` sees the root
/// first, so a failure on the root yields nothing else.
pub(super) fn eval_paths(
    pred: Option<&Filter>,
    input: &Value,
    env: &[(String, Value)],
) -> Result<Vec<Value>, Failure> {
    // `paths` with no filter is every path `..` reaches except the value
    // itself. A direct walk records those paths. The filtered form still
    // goes through `path`, because the condition can fail part way.
    if pred.is_none() {
        return Ok(gather_paths(input));
    }
    let walked = walk_paths(&Filter::Recurse, input, env);
    let mut out = Vec::new();
    for steps in walked.steps {
        if let Some(pred) = pred {
            let at = value_at(input, &steps).map_err(|message| fresh(out.clone(), message))?;
            let (flags, err) = parts(eval_in(pred, &at, env));
            for flag in flags {
                if truthy(&flag) && !steps.is_empty() {
                    out.push(render_steps(&steps));
                }
            }
            if let Some(failure) = err {
                return Err(with_partial(out, failure));
            }
        } else if !steps.is_empty() {
            out.push(render_steps(&steps));
        }
    }
    match walked.failure {
        Some(failure) => Err(with_partial(out, failure)),
        None => Ok(out),
    }
}

pub(super) fn eval_getpath(
    arg: &Filter,
    input: &Value,
    env: &[(String, Value)],
) -> Result<Vec<Value>, Failure> {
    map_arg(arg, input, env, |path| get_value(input, path))
}

pub(super) fn eval_setpath(
    path_f: &Filter,
    value_f: &Filter,
    input: &Value,
    env: &[(String, Value)],
) -> Result<Vec<Value>, Failure> {
    let (values, value_err) = parts(eval_in(value_f, input, env));
    let mut out = Vec::new();
    for value in values {
        let (paths, path_err) = parts(eval_in(path_f, input, env));
        for path in paths {
            match set_value(input, &path, &value) {
                Ok(produced) => out.push(produced),
                Err(message) => return Err(fresh(out, message)),
            }
        }
        if let Some(failure) = path_err {
            return Err(with_partial(out, failure));
        }
    }
    finish(out, value_err)
}

pub(super) fn eval_delpaths(
    arg: &Filter,
    input: &Value,
    env: &[(String, Value)],
) -> Result<Vec<Value>, Failure> {
    map_arg(arg, input, env, |paths| delete_paths(input, paths))
}

fn map_arg(
    arg: &Filter,
    input: &Value,
    env: &[(String, Value)],
    apply: impl Fn(&Value) -> Result<Value, String>,
) -> Result<Vec<Value>, Failure> {
    let (values, err) = parts(eval_in(arg, input, env));
    let mut out = Vec::new();
    for value in values {
        match apply(&value) {
            Ok(produced) => out.push(produced),
            Err(message) => return Err(fresh(out, message)),
        }
    }
    finish(out, err)
}

struct Walk {
    steps: Vec<Vec<Step>>,
    failure: Option<Failure>,
}

fn walk_paths(filter: &Filter, input: &Value, env: &[(String, Value)]) -> Walk {
    let next = Cell::new(2);
    let line = Cell::new(1);
    let ctx = Ctx {
        root: input,
        next: &next,
        env,
        pbinds: &[],
    };
    let mut steps = Vec::new();
    let failure = match stream_paths(
        filter,
        input,
        &[],
        &line,
        1,
        &ctx,
        &mut |out| match path_steps(input, out) {
            Ok(one) => {
                steps.push(one);
                Ok(())
            }
            Err(value) => Err(PathStop::Rejected(fail(format!(
                "Invalid path expression with result {}",
                clipped(&value, 29)
            )))),
        },
    ) {
        Ok(()) => None,
        Err(PathStop::Filter(failure) | PathStop::Rejected(failure)) => Some(failure),
    };
    Walk { steps, failure }
}

/// `path` checks each output as it is produced. A value that is not a path
/// stops the walk, so a later slice bound is not run. Value filters and
/// updates do not use this.
fn stream_paths(
    filter: &Filter,
    input: &Value,
    path: &[Step],
    line: &Cell<u64>,
    input_gen: u64,
    ctx: &Ctx,
    emit: &mut dyn FnMut(POut) -> Result<(), PathStop>,
) -> Result<(), PathStop> {
    match filter {
        Filter::Slice {
            base,
            start,
            end,
            optional,
        } => stream_slice(
            base.as_deref(),
            start.as_deref(),
            end.as_deref(),
            *optional,
            input,
            path,
            line,
            input_gen,
            ctx,
            emit,
        ),
        Filter::At {
            base,
            key,
            optional,
        } => stream_at(
            base, key, *optional, input, path, line, input_gen, ctx, emit,
        ),
        Filter::Comma(left, right) => {
            let saved = line.get();
            let left_cell = Cell::new(saved);
            stream_paths(left, input, path, &left_cell, input_gen, ctx, emit)?;
            let right_cell = Cell::new(saved);
            stream_paths(right, input, path, &right_cell, input_gen, ctx, emit)
        }
        // `match | .[]` reports the match array. The ordinary pipe walk
        // would turn that into a later index error.
        Filter::Pipe(left, right) if !matches!(left.as_ref(), Filter::MatchImpl { .. }) => {
            stream_pipe(left, right, input, path, line, input_gen, ctx, emit)
        }
        // `(expr)?` swallows a filter error. It does not swallow `path`
        // rejecting a value, and it does not run a later bound after that.
        Filter::Optional(inner)
            if matches!(
                inner.as_ref(),
                Filter::At { optional: true, .. } | Filter::Slice { optional: true, .. }
            ) =>
        {
            stream_paths(inner, input, path, line, input_gen, ctx, emit)
        }
        Filter::Optional(inner) => {
            match stream_paths(inner, input, path, line, input_gen, ctx, emit) {
                Err(PathStop::Filter(_)) => Ok(()),
                other => other,
            }
        }
        _ => stream_run(filter, input, path, line, input_gen, ctx, emit),
    }
}

fn stream_run(
    filter: &Filter,
    input: &Value,
    path: &[Step],
    line: &Cell<u64>,
    input_gen: u64,
    ctx: &Ctx,
    emit: &mut dyn FnMut(POut) -> Result<(), PathStop>,
) -> Result<(), PathStop> {
    match run(filter, input, path, line, input_gen, ctx) {
        Ok(outs) => {
            for out in outs {
                emit(out)?;
            }
            Ok(())
        }
        Err(err) => {
            let (partial, failure) = split_err(err);
            for out in partial {
                emit(out)?;
            }
            Err(PathStop::Filter(failure))
        }
    }
}

fn stream_pipe(
    left: &Filter,
    right: &Filter,
    input: &Value,
    path: &[Step],
    line: &Cell<u64>,
    input_gen: u64,
    ctx: &Ctx,
    emit: &mut dyn FnMut(POut) -> Result<(), PathStop>,
) -> Result<(), PathStop> {
    let left_cell = Cell::new(line.get());
    stream_paths(
        left,
        input,
        path,
        &left_cell,
        input_gen,
        ctx,
        &mut |left_out| {
            let (gen, next_gen) = if left_out.carrier {
                (left_out.value_gen, left_out.value_gen)
            } else {
                (left_out.line, 0)
            };
            let cell = Cell::new(gen);
            stream_paths(
                right,
                &left_out.value,
                &left_out.path,
                &cell,
                next_gen,
                ctx,
                emit,
            )
        },
    )
}

fn stream_at(
    base: &Filter,
    key: &Filter,
    optional: bool,
    input: &Value,
    path: &[Step],
    line: &Cell<u64>,
    input_gen: u64,
    ctx: &Ctx,
    emit: &mut dyn FnMut(POut) -> Result<(), PathStop>,
) -> Result<(), PathStop> {
    let saved = line.get();
    let slot: RefCell<Option<PathStop>> = RefCell::new(None);
    // One key at a time. A value that is not a path stops before a later key,
    // so `.[1,(input)] | 9` does not read `input`.
    let result =
        stream::for_each_value(key, input, ctx.env, |key_value| {
            line.set(saved);
            let sent = stream_paths(base, input, path, line, input_gen, ctx, &mut |base_out| {
                match index_out(&base_out, key_value, ctx) {
                    Ok(hit) => emit(hit),
                    Err(message) if optional && skips_failed_index(&message) => {
                        let _ = message;
                        Ok(())
                    }
                    Err(message) => Err(PathStop::Filter(fail(message))),
                }
            });
            match sent {
                Ok(()) => Ok(false),
                Err(err) => {
                    *slot.borrow_mut() = Some(err);
                    Ok(true)
                }
            }
        });
    if let Some(err) = slot.into_inner() {
        return Err(err);
    }
    match result {
        Ok(_) => Ok(()),
        Err(failure) => Err(PathStop::Filter(failure)),
    }
}

fn stream_slice(
    base: Option<&Filter>,
    start: Option<&Filter>,
    end: Option<&Filter>,
    optional: bool,
    input: &Value,
    path: &[Step],
    line: &Cell<u64>,
    input_gen: u64,
    ctx: &Ctx,
    emit: &mut dyn FnMut(POut) -> Result<(), PathStop>,
) -> Result<(), PathStop> {
    let identity = Filter::Identity;
    let base = base.unwrap_or(&identity);
    let saved = line.get();
    let slot: RefCell<Option<PathStop>> = RefCell::new(None);
    let walked = walk_slice_pairs(start, end, input, ctx.env, |start_value, end_value| {
        line.set(saved);
        let sent = stream_paths(base, input, path, line, input_gen, ctx, &mut |base_out| {
            if !base_out.carrier {
                let elem = shown_slice(start_value, end_value);
                return Err(PathStop::Filter(fail(near_access(&elem, &base_out.value))));
            }
            match &base_out.value {
                Value::Array(_) | Value::String(_) => {
                    if require_slice_index(start_value).is_err()
                        || require_slice_index(end_value).is_err()
                    {
                        if optional {
                            return Ok(());
                        }
                        require_slice_index(start_value).map_err(PathStop::Filter)?;
                        require_slice_index(end_value).map_err(PathStop::Filter)?;
                    }
                }
                Value::Null => {}
                other => {
                    if optional {
                        return Ok(());
                    }
                    return Err(PathStop::Filter(fail(format!(
                        "Cannot index {} with object",
                        other.type_name()
                    ))));
                }
            }
            let step = Step::Slice {
                start: start_value.clone(),
                end: end_value.clone(),
            };
            let child = child_at(&base_out.value, &step)
                .map_err(|message| PathStop::Filter(fail(message)))?;
            let mut full = base_out.path;
            full.push(step);
            emit(taken(child, full, line, ctx.next))
        });
        match sent {
            Ok(()) => Ok(()),
            Err(failure) => {
                *slot.borrow_mut() = Some(failure);
                Err(fail(String::new()))
            }
        }
    });
    if let Some(failure) = slot.into_inner() {
        return Err(failure);
    }
    match walked {
        Ok(()) => Ok(()),
        Err(failure) => Err(PathStop::Filter(failure)),
    }
}

/// A path jq will print. A broken value is the `Err` payload.
fn path_steps(root: &Value, out: POut) -> Result<Vec<Step>, Value> {
    if out.force_broken {
        if out.path.iter().any(|step| matches!(step, Step::Huge(_))) {
            return Ok(out.path);
        }
        return Err(out.value);
    }
    if out.carrier {
        return Ok(out.path);
    }
    if matches!(out.value, Value::Null | Value::Bool(_)) {
        if let Ok(at) = value_at(root, &out.path) {
            if same_boolish(&out.value, &at) {
                return Ok(out.path);
            }
        }
    }
    Err(out.value)
}

/// Paths of every value inside `input`, in the order `..` visits them.
/// The path of `input` itself is left out.
fn gather_paths(input: &Value) -> Vec<Value> {
    struct Frame {
        value: Value,
        next: usize,
        /// Length of `path` before this value's own component was pushed.
        base: usize,
        seen: bool,
        emit: bool,
    }
    let mut out = Vec::new();
    let mut path: Vec<Value> = Vec::new();
    let mut stack = vec![Frame {
        value: input.clone(),
        next: 0,
        base: 0,
        seen: false,
        emit: false,
    }];
    while let Some(frame) = stack.last_mut() {
        if !frame.seen {
            frame.seen = true;
            if frame.emit {
                out.push(Value::array(path.clone()));
            }
        }
        let next = frame.next;
        if let Some((child, component)) = path_child(&frame.value, next) {
            frame.next = next + 1;
            let base = path.len();
            path.push(component);
            stack.push(Frame {
                value: child,
                next: 0,
                base,
                seen: false,
                emit: true,
            });
        } else {
            path.truncate(frame.base);
            stack.pop();
        }
    }
    out
}

/// The child at `index`, and the path component that names it.
fn path_child(value: &Value, index: usize) -> Option<(Value, Value)> {
    match value {
        Value::Array(items) => {
            let child = items.get(index)?.clone();
            Some((
                child,
                Value::Number(crate::value::Text::from(index.to_string())),
            ))
        }
        Value::Object(items) => {
            let (key, child) = items.get(index)?;
            Some((
                child.clone(),
                Value::String(crate::value::Text::from(key.clone())),
            ))
        }
        _ => None,
    }
}

fn render_steps(steps: &[Step]) -> Value {
    Value::array(steps.iter().map(render_step).collect())
}

fn render_step(step: &Step) -> Value {
    match step {
        Step::Key(key) => Value::String(crate::value::Text::from(key.clone())),
        Step::Index { text, .. } | Step::Huge(text) => {
            Value::Number(crate::value::Text::from(text.clone()))
        }
        Step::Raw(value) => value.clone(),
        Step::Slice { start, end } => shown_slice(start, end),
    }
}

fn render_bound(bound: &Value) -> Value {
    match bound {
        Value::Number(text) => Value::Number(crate::value::Text::from(canonical_number(text))),
        other => other.clone(),
    }
}

fn run_getpath(
    arg: &Filter,
    input: &Value,
    path: &[Step],
    line: &Cell<u64>,
    input_gen: u64,
    ctx: &Ctx,
) -> Result<Vec<POut>, PathErr> {
    let saved = line.get();
    let (paths, err) = parts(eval_in(arg, input, ctx.env));
    line.set(saved);
    let mut out = Vec::new();
    for path_value in paths {
        line.set(saved);
        let base = pass(input, path, line, input_gen, ctx.root);
        match extend_get(&base, &path_value, ctx) {
            Ok(hit) => out.push(hit),
            Err(message) => {
                return Err(PathErr {
                    partial: out,
                    message,
                    caught: None,
                    stop: None,
                })
            }
        }
    }
    finish_outs(out, err)
}

/// A carrier grows by each component. Anything else is an ordinary value,
/// and `path` then rejects it when it was not reached from the input.
fn extend_get(base: &POut, path_value: &Value, ctx: &Ctx) -> Result<POut, String> {
    let Value::Array(keys) = path_value else {
        return Err("Path must be specified as an array".to_string());
    };
    if !base.carrier {
        let value = get_along(&base.value, keys)?;
        return Ok(constant(value, &base.path, base.line, ctx.root));
    }
    let mut cur = base.clone();
    for key in keys {
        cur = push_get(&cur, key, ctx)?;
    }
    Ok(cur)
}

/// `getpath` keeps the key it was given. A slice object is stored whole so
/// `path` prints its keys in the order they were written.
fn push_get(base: &POut, key: &Value, ctx: &Ctx) -> Result<POut, String> {
    if !base.carrier {
        return Err(near_access(key, &base.value));
    }
    match key {
        Value::Array(_) if matches!(base.value, Value::Array(_)) => Ok(adopt(
            base,
            Step::Raw(key.clone()),
            Value::array(Vec::new()),
            ctx,
        )),
        Value::Object(_) if matches!(base.value, Value::Null) => {
            Ok(adopt(base, Step::Raw(key.clone()), Value::Null, ctx))
        }
        Value::Object(_) if matches!(base.value, Value::Array(_) | Value::String(_)) => {
            let child = get_one(&base.value, key)?;
            Ok(adopt(base, Step::Raw(key.clone()), child, ctx))
        }
        _ => index_out(base, key, ctx),
    }
}

fn adopt(base: &POut, step: Step, child: Value, ctx: &Ctx) -> POut {
    let gen = alloc(ctx.next);
    let mut path = base.path.clone();
    path.push(step);
    POut {
        value: child,
        path,
        value_gen: gen,
        line: gen,
        carrier: true,
        force_broken: false,
    }
}

fn get_value(value: &Value, path: &Value) -> Result<Value, String> {
    let Value::Array(keys) = path else {
        return Err("Path must be specified as an array".to_string());
    };
    get_along(value, keys)
}

fn get_along(value: &Value, keys: &[Value]) -> Result<Value, String> {
    let mut cur = value.clone();
    for key in keys {
        cur = get_one(&cur, key)?;
    }
    Ok(cur)
}

fn get_one(value: &Value, key: &Value) -> Result<Value, String> {
    match key {
        Value::String(name) => match value {
            Value::Null => Ok(Value::Null),
            Value::Object(items) => Ok(items
                .iter()
                .find(|(item, _)| item == name.as_str())
                .map(|(_, item)| item.clone())
                .unwrap_or(Value::Null)),
            other => Err(format!(
                "Cannot index {} with string \"{name}\"",
                other.type_name()
            )),
        },
        Value::Number(text) => {
            let shown = canonical_number(text);
            let index = array_index(&shown);
            match value {
                Value::Null => Ok(Value::Null),
                Value::Array(items) => match index {
                    Some(index) => Ok(nth_value(items, index)),
                    None => Ok(Value::Null),
                },
                other => Err(format!("Cannot index {} with number", other.type_name())),
            }
        }
        Value::Object(fields) => {
            if matches!(value, Value::Null) {
                return Ok(Value::Null);
            }
            if matches!(value, Value::Array(_) | Value::String(_)) {
                let (start, end) = slice_from_object(fields)?;
                let sliced = slice_value(value, start.as_deref(), end.as_deref())
                    .map_err(|failure| failure.message)?;
                return Ok(sliced.into_iter().next().unwrap_or(Value::Null));
            }
            Err(format!("Cannot index {} with object", value.type_name()))
        }
        Value::Array(_) if matches!(value, Value::Array(_)) => Ok(Value::array(Vec::new())),
        other => Err(format!(
            "Cannot index {} with {}",
            value.type_name(),
            other.type_name()
        )),
    }
}

fn set_value(value: &Value, path: &Value, new_value: &Value) -> Result<Value, String> {
    let Value::Array(keys) = path else {
        return Err("Path must be specified as an array".to_string());
    };
    if keys.is_empty() {
        return Ok(new_value.clone());
    }
    let mut steps = Vec::new();
    let mut cur = value.clone();
    for key in keys {
        match real_step(&cur, key)? {
            Real::Huge(text) => return Err(huge_index_error(&text)),
            Real::Done(step, child) => {
                steps.push(step);
                cur = child;
            }
        }
    }
    let mut edit = Edit::Pure(value.clone());
    write_path(&mut edit, &steps, &Write::Put(new_value.clone()))?;
    Ok(freeze(&edit))
}

fn delete_paths(value: &Value, spec: &Value) -> Result<Value, String> {
    let Value::Array(paths) = spec else {
        return Err("Paths must be specified as an array".to_string());
    };
    let mut all = Vec::new();
    for path in paths {
        let Value::Array(keys) = path else {
            return Err(format!(
                "Path must be specified as array, not {}",
                path.type_name()
            ));
        };
        if let Some(steps) = delete_steps(value, keys)? {
            all.push(steps);
        }
    }
    let mut edit = Edit::Pure(value.clone());
    write_all(&mut edit, &all, &Write::Remove)?;
    Ok(freeze(&edit))
}

/// `None` deletes nothing. The last component uses jq's delete wording.
/// Earlier components use the same errors as `getpath`.
fn delete_steps(value: &Value, keys: &[Value]) -> Result<Option<Vec<Step>>, String> {
    if keys.is_empty() {
        return Ok(Some(Vec::new()));
    }
    let mut cur = value.clone();
    let mut steps = Vec::new();
    let mut detached = false;
    for (index, key) in keys.iter().enumerate() {
        let last = index + 1 == keys.len();
        if last {
            return match delete_leaf(&cur, key)? {
                Leaf::Skip => Ok(None),
                Leaf::Step(step) => {
                    if detached {
                        Ok(None)
                    } else {
                        steps.push(step);
                        Ok(Some(steps))
                    }
                }
            };
        }
        if let Value::Array(_) = key {
            return match &cur {
                Value::Array(_) => Err("Cannot update field at array index of array".to_string()),
                other => Err(format!("Cannot index {} with array", other.type_name())),
            };
        }
        let (step, child) = read_step(&cur, key)?;
        if matches!(step, Step::Huge(_)) {
            detached = true;
            cur = Value::Null;
            continue;
        }
        if !detached {
            steps.push(step);
        }
        cur = child;
    }
    Ok(Some(steps))
}

enum Leaf {
    Skip,
    Step(Step),
}

fn delete_leaf(value: &Value, key: &Value) -> Result<Leaf, String> {
    match value {
        Value::Null => Ok(Leaf::Skip),
        Value::Array(_) => match key {
            Value::Number(text) => {
                let shown = canonical_number(text);
                match array_index(&shown) {
                    Some(index) => Ok(Leaf::Step(index_step(index, &shown))),
                    None => Ok(Leaf::Skip),
                }
            }
            Value::Object(fields) => {
                let (start, end) = slice_from_object(fields)?;
                Ok(Leaf::Step(Step::Slice {
                    start: as_bound(start),
                    end: as_bound(end),
                }))
            }
            other => Err(format!(
                "Cannot delete {} element of array",
                other.type_name()
            )),
        },
        Value::Object(_) => match key {
            Value::String(name) => Ok(Leaf::Step(Step::Key(name.to_string()))),
            other => Err(format!(
                "Cannot delete {} field of object",
                other.type_name()
            )),
        },
        other => Err(format!("Cannot delete fields from {}", other.type_name())),
    }
}

/// One `getpath` step. A number that does not fit yields `Huge` and `null`.
fn read_step(value: &Value, key: &Value) -> Result<(Step, Value), String> {
    match key {
        Value::String(name) => {
            let child = match value {
                Value::Null | Value::Object(_) => object_field(value, name),
                other => {
                    return Err(format!(
                        "Cannot index {} with string \"{name}\"",
                        other.type_name()
                    ))
                }
            };
            Ok((Step::Key(name.to_string()), child))
        }
        Value::Number(text) => {
            let shown = canonical_number(text);
            match array_index(&shown) {
                Some(index) => {
                    let child = match value {
                        Value::Null => Value::Null,
                        Value::Array(items) => nth_value(items, index),
                        other => {
                            return Err(format!("Cannot index {} with number", other.type_name()))
                        }
                    };
                    Ok((index_step(index, &shown), child))
                }
                None => {
                    if matches!(value, Value::Null | Value::Array(_)) {
                        Ok((Step::Huge(shown), Value::Null))
                    } else {
                        Err(format!("Cannot index {} with number", value.type_name()))
                    }
                }
            }
        }
        Value::Object(fields) => {
            if !matches!(value, Value::Null | Value::Array(_) | Value::String(_)) {
                return Err(format!("Cannot index {} with object", value.type_name()));
            }
            let (start, end) = slice_from_object(fields)?;
            let step = Step::Slice {
                start: as_bound(start.clone()),
                end: as_bound(end.clone()),
            };
            let child = if matches!(value, Value::Null) {
                Value::Null
            } else {
                let sliced = slice_value(value, start.as_deref(), end.as_deref())
                    .map_err(|failure| failure.message)?;
                sliced.into_iter().next().unwrap_or(Value::Null)
            };
            Ok((step, child))
        }
        Value::Array(_) if matches!(value, Value::Array(_)) => {
            Err("Cannot update field at array index of array".to_string())
        }
        other => Err(format!(
            "Cannot index {} with {}",
            value.type_name(),
            other.type_name()
        )),
    }
}
