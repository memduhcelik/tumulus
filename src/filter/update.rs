//! Path updates. A path is collected from the input, then each write is applied
//! in that order. Array indexes keep their original slots until the update
//! finishes, so a later index is not shifted by an earlier deletion.

use std::cell::Cell;

use crate::number::array_index;
use crate::value::Value;

use super::stream::{self, LimitHow, NthSpan};
use super::{
    add_values, bare_fail, canonical_number, child_scope, clipped, div_values, eval_in, fail,
    finish, fresh, mul_values, parts, rem_values, shown, slice as slice_value, slice_range,
    sub_values, truthy, with_partial, ArrayIndex, Failure, Filter, PatField, Pattern, UpdateOp,
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
    let (rhs, rhs_err) = parts(eval_in(expr, input, env));
    if rhs.is_empty() {
        return finish(Vec::new(), rhs_err);
    }
    let paths = collect(path, input, env)?;
    if paths.is_empty() {
        let outs = vec![input.clone(); rhs.len()];
        return finish(outs, rhs_err);
    }
    let mut outs = Vec::new();
    for value in rhs {
        let produced = match op {
            UpdateOp::Assign => assign_one(&paths, input, value),
            _ => combine_one(op, &paths, input, value),
        };
        match produced {
            Ok(value) => outs.push(value),
            Err(message) => return Err(fresh(outs, message)),
        }
    }
    finish(outs, rhs_err)
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
                    clipped(&value, 29)
                )));
            }
            Found::NanIndex => {
                return Err(fail("Cannot set array element at NaN index".to_string()));
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
    /// The string is the spelling `path` prints.
    Huge(String),
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
                match classify_one(input, out) {
                    Found::Broken(value) => {
                        return Err(fail(format!(
                            "Invalid path expression with result {}",
                            clipped(&value, 29)
                        )));
                    }
                    Found::NanIndex => {
                        return Err(fail("Cannot set array element at NaN index".to_string()));
                    }
                    Found::Path(_) => {}
                }
            }
            Err(Failure {
                partial: Vec::new(),
                message: err.message,
                caught: err.caught,
            })
        }
    }
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
        Filter::Slice { start, end } => run_slice(start, end, input, path, line, input_gen, ctx),
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
        Filter::Try { body, handler } => {
            run_try(body, handler.as_deref(), input, path, line, input_gen, ctx)
        }
        // `first` and `nth` keep the path of the value they select. `last(expr)`
        // is the value itself, so it falls through and is not a path.
        Filter::First(None) => index_here(
            &Value::Number("0".to_string()),
            input,
            path,
            line,
            input_gen,
            ctx,
        ),
        Filter::Last(None) => index_here(
            &Value::Number("-1".to_string()),
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
        other => run_fallback(other, input, path, line, ctx),
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
            Ok(_) => match sub_values(count, &Value::Number("1".to_string())) {
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
    }
}

fn attach(mut done: Vec<POut>, err: PathErr) -> PathErr {
    done.extend(err.partial);
    PathErr {
        partial: done,
        message: err.message,
        caught: err.caught,
    }
}

fn split_err(mut err: PathErr) -> (Vec<POut>, Failure) {
    (
        std::mem::take(&mut err.partial),
        Failure {
            partial: Vec::new(),
            message: err.message,
            caught: err.caught,
        },
    )
}

fn finish_outs(out: Vec<POut>, err: Option<Failure>) -> Result<Vec<POut>, PathErr> {
    match err {
        Some(failure) => Err(PathErr {
            partial: out,
            message: failure.message,
            caught: failure.caught,
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
        None => Value::String(err.message),
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
    text: &str,
    input: &Value,
    path: &[Step],
    line: &Cell<u64>,
    input_gen: u64,
    ctx: &Ctx,
) -> Result<Vec<POut>, PathErr> {
    match index {
        ArrayIndex::At(_) => {
            let key = Value::Number(text.to_string());
            index_here(&key, input, path, line, input_gen, ctx)
        }
        ArrayIndex::OutOfRange => {
            if !carrier_now(line, input_gen) {
                return Err(bare(near_access(&Value::Null, input)));
            }
            if matches!(input, Value::Null | Value::Array(_)) {
                Ok(vec![huge_out(path, text, line)])
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
                    let (message, caught) = match key_err.clone() {
                        Some(failure) => (failure.message, failure.caught),
                        None => (message, None),
                    };
                    return Err(PathErr {
                        partial: out,
                        message,
                        caught,
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
        Real::Huge(text) => Ok(vec![huge_out(path, &text, line)]),
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
        Real::Huge(text) => Ok(huge_out(&base.path, &text, &Cell::new(base.line))),
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

fn huge_out(path: &[Step], text: &str, line: &Cell<u64>) -> POut {
    let mut full = path.to_vec();
    full.push(Step::Huge(text.to_string()));
    POut {
        value: Value::Null,
        path: full,
        value_gen: 0,
        line: line.get(),
        carrier: false,
        force_broken: true,
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
                Step::Key(name.clone()),
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
                caught: failure.caught,
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
        Step::Index { at, .. } => write_index(edit, *at, &path[1..], write),
        Step::Huge(_) => Err("Array index too large".to_string()),
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
            write_slice(edit, &start, &end, rest, write)
        }
        _ => {
            let current = freeze(edit);
            match real_step(&current, key)? {
                Real::Huge(_) => Err("Array index too large".to_string()),
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
        (_, Step::Raw(key)) => {
            let current = freeze(edit);
            let got = get_one(&current, key)?;
            return if path.len() == 1 {
                Ok(got)
            } else {
                value_at(&got, &path[1..])
            };
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
        (Edit::Arr(_), Step::Huge(_)) => Err("Array index too large".to_string()),
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
    let (outs, err) = match run(filter, input, &[], &line, 1, &ctx) {
        Ok(outs) => (outs, None),
        Err(err) => {
            let (partial, failure) = split_err(err);
            (partial, Some(failure))
        }
    };
    let mut steps = Vec::new();
    for out in outs {
        match path_steps(input, out) {
            Ok(one) => steps.push(one),
            Err(value) => {
                return Walk {
                    steps,
                    failure: Some(fail(format!(
                        "Invalid path expression with result {}",
                        clipped(&value, 29)
                    ))),
                };
            }
        }
    }
    Walk {
        steps,
        failure: err,
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

fn render_steps(steps: &[Step]) -> Value {
    Value::Array(steps.iter().map(render_step).collect())
}

fn render_step(step: &Step) -> Value {
    match step {
        Step::Key(key) => Value::String(key.clone()),
        Step::Index { text, .. } | Step::Huge(text) => Value::Number(text.clone()),
        Step::Raw(value) => value.clone(),
        Step::Slice { start, end } => Value::Object(vec![
            ("start".to_string(), render_bound(start)),
            ("end".to_string(), render_bound(end)),
        ]),
    }
}

fn render_bound(bound: &Option<String>) -> Value {
    match bound {
        None => Value::Null,
        Some(text) => Value::Number(canonical_number(text)),
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
            Value::Array(Vec::new()),
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
                .find(|(item, _)| item == name)
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
        Value::Array(_) if matches!(value, Value::Array(_)) => Ok(Value::Array(Vec::new())),
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
            Real::Huge(_) => return Err("Array index too large".to_string()),
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
                Ok(Leaf::Step(Step::Slice { start, end }))
            }
            other => Err(format!(
                "Cannot delete {} element of array",
                other.type_name()
            )),
        },
        Value::Object(_) => match key {
            Value::String(name) => Ok(Leaf::Step(Step::Key(name.clone()))),
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
            Ok((Step::Key(name.clone()), child))
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
                start: start.clone(),
                end: end.clone(),
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
