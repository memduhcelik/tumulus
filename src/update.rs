//! Path updates. A path is collected from the input, then each write is applied
//! in that order. Array indexes keep their original slots until the update
//! finishes, so a later index is not shifted by an earlier deletion.

use std::cell::Cell;

use crate::number::array_index;
use crate::print::{self, Layout, Style};
use crate::value::Value;

use super::{
    add_values, child_scope, clipped, div_values, eval_in, fail, mul_values, parts, rem_values,
    shown, slice_range, sub_values, truthy, ArrayIndex, Failure, Filter, PatField, Pattern,
    UpdateOp,
};

enum Found {
    Path(Vec<Step>),
    /// A value that was computed instead of reached through the input.
    Broken(Value),
}

#[derive(Clone)]
enum Step {
    Key(String),
    Index(i64),
    Slice {
        start: Option<String>,
        end: Option<String>,
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
    /// A slice wrote these values in place of this original slot.
    Run(Vec<Edit>),
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
        let paths = collect(path, input, env)?;
        if paths.is_empty() {
            // Nothing was reached, so the filter is not run.
            return Ok(vec![input.clone()]);
        }
        return modify(&paths, expr, input, env);
    }
    // `=` and the arithmetic updates run the right-hand side first. An empty
    // stream yields nothing and does not look at the path. A failure while
    // producing a value is reported before a bad path is.
    let (rhs, rhs_err) = match eval_in(expr, input, env) {
        Ok(values) => (values, None),
        Err(failure) => (failure.partial, Some(failure.message)),
    };
    if rhs.is_empty() {
        return match rhs_err {
            Some(message) => Err(fail(message)),
            None => Ok(Vec::new()),
        };
    }
    let paths = collect(path, input, env)?;
    if paths.is_empty() {
        let outs = vec![input.clone(); rhs.len()];
        return match rhs_err {
            Some(message) => Err(Failure {
                partial: outs,
                message,
            }),
            None => Ok(outs),
        };
    }
    let mut outs = Vec::new();
    for value in rhs {
        let produced = match op {
            UpdateOp::Assign => assign_one(&paths, input, value),
            _ => combine_one(op, &paths, input, value),
        };
        match produced {
            Ok(value) => outs.push(value),
            Err(message) => {
                return Err(Failure {
                    partial: outs,
                    message,
                })
            }
        }
    }
    match rhs_err {
        Some(message) => Err(Failure {
            partial: outs,
            message,
        }),
        None => Ok(outs),
    }
}

fn collect(
    path: &Filter,
    input: &Value,
    env: &[(String, Value)],
) -> Result<Vec<Vec<Step>>, Failure> {
    let mut paths = Vec::new();
    for item in locate(path, input, env)? {
        match item {
            Found::Broken(value) => {
                return Err(fail(format!(
                    "Invalid path expression with result {}",
                    compact(&value)
                )));
            }
            Found::Path(steps) => paths.push(steps),
        }
    }
    Ok(paths)
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

fn combine_one(
    op: UpdateOp,
    paths: &[Vec<Step>],
    input: &Value,
    right: Value,
) -> Result<Value, String> {
    let mut edit = Edit::Pure(input.clone());
    for path in paths {
        let left = read_edit(&edit, path)?;
        let value = match op {
            UpdateOp::Add => add_values(&left, &right)?,
            UpdateOp::Sub => sub_values(&left, &right)?,
            UpdateOp::Mul => mul_values(&left, &right)?,
            UpdateOp::Div => div_values(&left, &right)?,
            UpdateOp::Rem => rem_values(&left, &right)?,
            UpdateOp::Alt => {
                if truthy(&left) {
                    left
                } else {
                    right.clone()
                }
            }
            UpdateOp::Assign | UpdateOp::Modify => unreachable!("arithmetic update"),
        };
        write_path(&mut edit, path, &Write::Put(value))?;
    }
    Ok(freeze(&edit))
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
                    return Err(Failure {
                        partial: Vec::new(),
                        message: failure.message,
                    });
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
            Err(failure) => Err(Failure {
                partial: Vec::new(),
                message: failure.message,
            }),
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
    Huge,
}

fn locate(filter: &Filter, input: &Value, env: &[(String, Value)]) -> Result<Vec<Found>, Failure> {
    let next = Cell::new(2);
    let line = Cell::new(1);
    let ctx = Ctx {
        root: input,
        next: &next,
        env,
        pbinds: &[],
    };
    match run(filter, input, &[], &line, 1, &ctx) {
        Ok(outs) => Ok(outs
            .into_iter()
            .map(|out| classify_one(input, out))
            .collect()),
        Err(err) => {
            for out in err.partial {
                if let Found::Broken(value) = classify_one(input, out) {
                    return Err(fail(format!(
                        "Invalid path expression with result {}",
                        compact(&value)
                    )));
                }
            }
            Err(fail(err.message))
        }
    }
}

fn classify_one(root: &Value, out: POut) -> Found {
    if out.force_broken {
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
        Filter::Index(index) => run_fixed_index(index, input, path, line, input_gen, ctx),
        Filter::Slice { start, end } => run_slice(start, end, input, path, line, input_gen, ctx),
        Filter::Iterate => run_iterate(input, path, line, input_gen, ctx),
        Filter::Recurse => run_recurse(input, path, line, input_gen, ctx),
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
            body,
        } => run_bind(expr, pattern, body, input, path, line, input_gen, ctx),
        Filter::At {
            base,
            key,
            optional,
        } => run_at(base, key, *optional, input, path, line, input_gen, ctx),
        Filter::Var(name) => match ctx.pbinds.iter().find(|bind| &bind.name == name) {
            Some(bind) => Ok(vec![emit_bind(bind, ctx.root)]),
            None => run_fallback(filter, input, path, line, ctx),
        },
        other => run_fallback(other, input, path, line, ctx),
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
    }
}

fn attach(mut done: Vec<POut>, err: PathErr) -> PathErr {
    done.extend(err.partial);
    PathErr {
        partial: done,
        message: err.message,
    }
}

fn finish_outs(out: Vec<POut>, err: Option<String>) -> Result<Vec<POut>, PathErr> {
    match err {
        Some(message) => Err(PathErr {
            partial: out,
            message,
        }),
        None => Ok(out),
    }
}

fn near_access(elem: &Value, value: &Value) -> String {
    format!(
        "Invalid path expression near attempt to access element {} of {}",
        clipped(elem, 14),
        clipped(value, 29)
    )
}

fn near_iter(value: &Value) -> String {
    format!(
        "Invalid path expression near attempt to iterate through {}",
        clipped(value, 29)
    )
}

fn slice_element(start: &Option<String>, end: &Option<String>) -> Value {
    Value::Object(vec![
        ("start".to_string(), bound_value(start)),
        ("end".to_string(), bound_value(end)),
    ])
}

fn bound_value(bound: &Option<String>) -> Value {
    match bound {
        Some(text) => Value::Number(text.clone()),
        None => Value::Null,
    }
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
        let elem = Value::String(key.to_string());
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
    input: &Value,
    path: &[Step],
    line: &Cell<u64>,
    input_gen: u64,
    ctx: &Ctx,
) -> Result<Vec<POut>, PathErr> {
    match index {
        ArrayIndex::At(index) => {
            let key = Value::Number(index.to_string());
            index_here(&key, input, path, line, input_gen, ctx)
        }
        ArrayIndex::OutOfRange => {
            if !carrier_now(line, input_gen) {
                return Err(bare(near_access(&Value::Null, input)));
            }
            if matches!(input, Value::Null | Value::Array(_)) {
                Ok(vec![huge_out(path, line)])
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
    start: &Option<String>,
    end: &Option<String>,
    input: &Value,
    path: &[Step],
    line: &Cell<u64>,
    input_gen: u64,
    ctx: &Ctx,
) -> Result<Vec<POut>, PathErr> {
    if !carrier_now(line, input_gen) {
        let elem = slice_element(start, end);
        return Err(bare(near_access(&elem, input)));
    }
    match input {
        Value::Null | Value::Array(_) | Value::String(_) => {
            let step = Step::Slice {
                start: start.clone(),
                end: end.clone(),
            };
            let child = child_at(input, &step).map_err(bare)?;
            let mut full = path.to_vec();
            full.push(step);
            Ok(vec![taken(child, full, line, ctx.next)])
        }
        other => Err(bare(format!(
            "Cannot index {} with object",
            other.type_name()
        ))),
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
                full.push(Step::Index(index as i64));
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
                child_path.push(Step::Index(index as i64));
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

fn run_optional(
    inner: &Filter,
    input: &Value,
    path: &[Step],
    line: &Cell<u64>,
    input_gen: u64,
    ctx: &Ctx,
) -> Result<Vec<POut>, PathErr> {
    // `.[expr]?` keeps the inner flag, so a key or base error still fails.
    // `(expr)?` swallows the error and keeps outputs from before it.
    if matches!(inner, Filter::At { optional: true, .. }) {
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
    let left_cell = Cell::new(line.get());
    let (lefts, left_err) = match run(left, input, path, &left_cell, input_gen, ctx) {
        Ok(outs) => (outs, None),
        Err(err) => (err.partial, Some(err.message)),
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
        Err(err) => (err.partial, Some(err.message)),
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
    body: &Filter,
    input: &Value,
    path: &[Step],
    line: &Cell<u64>,
    input_gen: u64,
    ctx: &Ctx,
) -> Result<Vec<POut>, PathErr> {
    let tail = Tail {
        body,
        orig_input: input,
        orig_input_gen: input_gen,
    };
    if matches!(expr, Filter::Identity) {
        let carrier = carrier_now(line, input_gen);
        return exec_pattern(
            pattern,
            input,
            path,
            line,
            carrier,
            input_gen,
            &[],
            Vec::new(),
            &tail,
            ctx,
        );
    }
    let saved = line.get();
    let saved_path = path.to_vec();
    let (values, err) = parts(eval_in(expr, input, ctx.env));
    line.set(saved);
    let mut out = Vec::new();
    for value in values {
        line.set(saved);
        match exec_pattern(
            pattern,
            &value,
            &saved_path,
            line,
            false,
            0,
            &[],
            Vec::new(),
            &tail,
            ctx,
        ) {
            Ok(more) => out.extend(more),
            Err(err) => return Err(attach(out, err)),
        }
    }
    finish_outs(out, err)
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
                Err(message) if optional => {
                    let _ = message;
                }
                Err(message) => {
                    let message = key_err.clone().unwrap_or(message);
                    return Err(PathErr {
                        partial: out,
                        message,
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
        Real::Huge => Ok(vec![huge_out(path, line)]),
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
        Real::Huge => Ok(huge_out(&base.path, &Cell::new(base.line))),
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

fn huge_out(path: &[Step], line: &Cell<u64>) -> POut {
    POut {
        value: Value::Null,
        path: path.to_vec(),
        value_gen: 0,
        line: line.get(),
        carrier: false,
        force_broken: true,
    }
}

fn real_step(container: &Value, key: &Value) -> Result<Real, String> {
    match key {
        Value::String(name) => match container {
            Value::Null | Value::Object(_) => Ok(Real::Done(
                Step::Key(name.clone()),
                object_field(container, name),
            )),
            other => Err(format!(
                "Cannot index {} with string \"{name}\"",
                other.type_name()
            )),
        },
        Value::Number(text) => {
            let Some(index) = array_index(text) else {
                return if matches!(container, Value::Null | Value::Array(_)) {
                    Ok(Real::Huge)
                } else {
                    Err(format!(
                        "Cannot index {} with number",
                        container.type_name()
                    ))
                };
            };
            match container {
                Value::Null => Ok(Real::Done(Step::Index(index), Value::Null)),
                Value::Array(items) => Ok(Real::Done(Step::Index(index), nth_value(items, index))),
                other => Err(format!("Cannot index {} with number", other.type_name())),
            }
        }
        Value::Object(fields) => match container {
            Value::Null | Value::Array(_) | Value::String(_) => {
                let (start, end) = slice_from_object(fields)?;
                let step = Step::Slice { start, end };
                let child = child_at(container, &step)?;
                Ok(Real::Done(step, child))
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
    let start = fields.iter().rev().find(|(key, _)| key == "start");
    let end = fields.iter().rev().find(|(key, _)| key == "end");
    let (Some((_, start)), Some((_, end))) = (start, end) else {
        return Err("Array/string slice indices must be integers".to_string());
    };
    Ok((bound_text(start)?, bound_text(end)?))
}

fn bound_text(value: &Value) -> Result<Option<String>, String> {
    match value {
        Value::Null => Ok(None),
        Value::Number(text) => Ok(Some(text.strip_prefix('\u{1}').unwrap_or(text).to_string())),
        _ => Err("Array/string slice indices must be integers".to_string()),
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
    let key = Value::Number((index - 1).to_string());
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
        Real::Huge => Err(bare(format!(
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
    let frame: Vec<(String, Value)> = binds
        .iter()
        .map(|bind| (bind.name.clone(), bind.value.clone()))
        .collect();
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
        (Step::Index(_), Value::Null) => Value::Null,
        (Step::Index(index), Value::Array(items)) => nth_value(items, *index),
        (Step::Index(_), other) => {
            return Err(format!("Cannot index {} with number", other.type_name()));
        }
        (Step::Slice { start, end }, _) => {
            let sliced = match super::slice(value, start.as_deref(), end.as_deref()) {
                Ok(values) => values,
                Err(failure) => return Err(failure.message),
            };
            let one = sliced.into_iter().next().unwrap_or(Value::Null);
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
        Step::Index(index) => write_index(edit, *index, &path[1..], write),
        Step::Slice { start, end } => write_slice(edit, start, end, &path[1..], write),
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
            let fields = std::mem::take(items)
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
    if matches!(arr.slots[position].body, Body::Run(_)) {
        return Err("Cannot index array with number".to_string());
    }
    let Body::Live(child) = &mut arr.slots[position].body else {
        return Ok(());
    };
    write_path(child, rest, write)
}

fn write_slice(
    edit: &mut Edit,
    start: &Option<String>,
    end: &Option<String>,
    rest: &[Step],
    write: &Write,
) -> Result<(), String> {
    if let Edit::Pure(Value::String(_)) = edit {
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
    let arr = force_array(edit)?;
    let (from, to) = slice_range(start.as_deref(), end.as_deref(), arr.slots.len());
    match write {
        Write::Remove => {
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
            let Value::Array(items) = value else {
                return Err("A slice of an array can only be assigned another array".to_string());
            };
            let run: Vec<Edit> = items.iter().cloned().map(Edit::Pure).collect();
            if from == to {
                if from >= arr.slots.len() {
                    arr.tail.extend(run);
                } else {
                    arr.slots[from].before.extend(run);
                }
                return Ok(());
            }
            arr.slots[from].body = Body::Run(run);
            for slot in &mut arr.slots[from + 1..to] {
                slot.before.clear();
                slot.body = Body::Gone;
            }
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
            let slots = std::mem::take(items)
                .into_iter()
                .map(|value| live(value))
                .collect();
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
        (Edit::Pure(value), _) => value_at(value, path),
        (Edit::Obj(fields), Step::Key(key)) => match fields.iter().find(|(name, _)| name == key) {
            Some((_, child)) => read_edit(child, &path[1..]),
            None => read_edit(&Edit::Pure(Value::Null), &path[1..]),
        },
        (Edit::Obj(_), Step::Index(_)) => Err("Cannot index object with number".to_string()),
        (Edit::Obj(_), Step::Slice { .. }) => Err("Cannot index object with object".to_string()),
        (Edit::Arr(arr), Step::Index(index)) => {
            let Some(position) = resolve_index(*index, arr.slots.len()) else {
                return Err("Out of bounds negative array index".to_string());
            };
            if position >= arr.slots.len() {
                return read_edit(&Edit::Pure(Value::Null), &path[1..]);
            }
            match &arr.slots[position].body {
                Body::Live(child) => read_edit(child, &path[1..]),
                Body::Gone => read_edit(&Edit::Pure(Value::Null), &path[1..]),
                Body::Run(items) => {
                    let value = Value::Array(items.iter().map(freeze).collect());
                    read_edit(&Edit::Pure(value), &path[1..])
                }
            }
        }
        (Edit::Arr(arr), Step::Slice { start, end }) => {
            let (from, to) = slice_range(start.as_deref(), end.as_deref(), arr.slots.len());
            let value = Value::Array(slice_slots(arr, from, to));
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
        Body::Run(items) => items.iter().map(freeze).collect(),
    }
}

fn freeze(edit: &Edit) -> Value {
    match edit {
        Edit::Pure(value) => value.clone(),
        Edit::Obj(fields) => Value::Object(
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
            Value::Array(items)
        }
    }
}

fn compact(value: &Value) -> String {
    let style = Style {
        layout: Layout::Compact,
        ..Style::default()
    };
    print::render(value, style)
}
