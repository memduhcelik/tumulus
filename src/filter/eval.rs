//! Running a filter: indexes, pipes, bindings, objects, and strings.

use std::cell::{Cell, RefCell};

use crate::date;
use crate::math;
use crate::number::{array_index, slice_bound};
use crate::print::{self, Layout, Style};
use crate::value::Value;

use super::ast::*;
use super::builtin::*;
use super::parse::key_message;
use super::value::*;

pub fn eval(filter: &Filter, input: &Value) -> Result<Vec<Value>, Failure> {
    eval_with(filter, input, &[])
}

/// `env` is the outer binding list. The first binding of a name wins.
pub fn eval_with(
    filter: &Filter,
    input: &Value,
    env: &[(String, Value)],
) -> Result<Vec<Value>, Failure> {
    let _scope = super::func::root_scope();
    eval_in(filter, input, env)
}

/// Run `filter` and hand each value to `on_value` before the next one is built.
/// `on_value` returns `true` to stop. A value already handed over is not in
/// the error's partial results.
pub fn eval_stream(
    filter: &Filter,
    input: &Value,
    env: &[(String, Value)],
    on_value: &mut dyn FnMut(&Value) -> bool,
) -> Result<(), Failure> {
    let _scope = super::func::root_scope();
    match super::stream::feed(filter, input, env, on_value) {
        Ok(_) => Ok(()),
        Err(failure) => Err(failure),
    }
}

pub(super) fn eval_in(
    filter: &Filter,
    input: &Value,
    env: &[(String, Value)],
) -> Result<Vec<Value>, Failure> {
    super::stream::collect(filter, input, env)
}

/// One filter, run to the end. `stream` stops a `pipe` or a `comma` early.
/// Calling this on those two goes through `stream` as well.
pub(super) fn produce(
    filter: &Filter,
    input: &Value,
    env: &[(String, Value)],
) -> Result<Vec<Value>, Failure> {
    match filter {
        Filter::Identity => Ok(vec![input.clone()]),
        Filter::Field(key) => index_string(input, key),
        Filter::Index { at, .. } => index_number(input, at),
        Filter::Slice { base, start, end } => eval_slice(
            base.as_deref(),
            start.as_deref(),
            end.as_deref(),
            input,
            env,
        ),
        Filter::Iterate => iterate(input),
        Filter::Recurse => Ok(recurse(input)),
        Filter::Optional(inner) => match eval_in(inner, input, env) {
            Ok(values) => Ok(values),
            Err(failure) if failure.stop.is_some() => Err(failure),
            Err(failure) => Ok(failure.partial),
        },
        Filter::Pipe(left, right) => super::stream::pipe_values(left, right, input, env),
        Filter::Comma(left, right) => super::stream::comma_values(left, right, input, env),
        Filter::Literal(value) => Ok(vec![value.clone()]),
        Filter::Interpolate(pieces) => eval_interpolate(pieces, input, env),
        Filter::Format(name) => super::builtin::eval_named_format(name, input),
        Filter::FormatString { name, pieces } => {
            super::builtin::eval_format_string(name, pieces, input, env)
        }
        Filter::Collect(inner) => match eval_in(inner, input, env) {
            Ok(items) => Ok(vec![Value::array(items)]),
            Err(failure) => Err(bare_fail(failure)),
        },
        Filter::Object(fields) => eval_object(fields, input, env),
        Filter::At {
            base,
            key,
            optional,
        } => eval_at(base, key, *optional, input, env),
        Filter::Bin { op, left, right } => eval_bin(*op, left, right, input, env),
        Filter::Neg(inner) => eval_neg(inner, input, env),
        Filter::Not => Ok(vec![Value::Bool(!truthy(input))]),
        Filter::And(left, right) => eval_and(left, right, input, env),
        Filter::Or(left, right) => eval_or(left, right, input, env),
        Filter::Alt(left, right) => eval_alt(left, right, input, env),
        Filter::If { cond, yes, no } => eval_if(cond, yes, no, input, env),
        Filter::Var(name) => match lookup(env, name) {
            Some(value) => Ok(vec![value.clone()]),
            None => Err(fail(format!("unbound {name}"))),
        },
        Filter::Here(line) => Ok(vec![crate::feed::here(*line)]),
        Filter::Input => one_read(crate::feed::read_one()),
        Filter::Inputs => all_reads(),
        Filter::InputFilename => Ok(vec![crate::feed::input_filename()]),
        Filter::Environ => Ok(vec![crate::feed::environ()]),
        Filter::Order { kind, by } => super::sort::eval_order(*kind, by.as_deref(), input, env),
        Filter::Bind {
            expr,
            pattern,
            alts,
            body,
        } => eval_bind(expr, pattern, alts, body, input, env),
        Filter::Reduce {
            stream,
            pattern,
            alts,
            init,
            update,
        } => eval_fold(stream, pattern, alts, init, update, None, false, input, env),
        Filter::Foreach {
            stream,
            pattern,
            alts,
            init,
            update,
            extract,
        } => eval_fold(
            stream,
            pattern,
            alts,
            init,
            update,
            extract.as_deref(),
            true,
            input,
            env,
        ),
        Filter::Label { slot, body } => eval_label(*slot, body, input, env),
        Filter::Break { slot, .. } => Err(break_failure(*slot)),
        Filter::Empty => Ok(Vec::new()),
        Filter::Length => eval_length(input),
        Filter::Keys => eval_keys(input),
        Filter::KeysUnsorted => super::builtin::eval_keys_unsorted(input),
        Filter::FromJson => super::builtin::eval_fromjson(input),
        Filter::Split(sep) => super::builtin::eval_split(sep, input, env),
        Filter::Contains(needle) => super::builtin::eval_contains(needle, input, env),
        Filter::Inside(haystack) => super::builtin::eval_inside(haystack, input, env),
        Filter::Flatten(depth) => super::builtin::eval_flatten(depth.as_deref(), input, env),
        Filter::Quant {
            all, stream, cond, ..
        } => super::builtin::eval_quant(*all, stream, cond, input, env),
        Filter::IsEmpty(expr) => super::builtin::eval_isempty(expr, input, env),
        Filter::In { src, stream } => {
            super::builtin::eval_member(src.as_deref(), stream, input, env)
        }
        Filter::IndexBy { stream, key } => {
            super::builtin::eval_index_by(stream.as_deref(), key, input, env)
        }
        Filter::MapValues(expr) => super::builtin::eval_map_values(expr, input, env),
        Filter::Has(arg) => eval_has(arg, input, env),
        Filter::Select(arg) => eval_select(arg, input, env),
        Filter::Map(arg) => eval_map(arg, input, env),
        Filter::Type => Ok(vec![Value::String(input.type_name().to_string())]),
        Filter::ToNumber => eval_tonumber(input),
        Filter::ToString => Ok(vec![Value::String(jq_text(input))]),
        Filter::ToJson => super::builtin::eval_tojson(input),
        Filter::Add => super::builtin::eval_add(input),
        Filter::ToEntries => super::builtin::eval_to_entries(input),
        Filter::FromEntries => super::builtin::eval_from_entries(input),
        Filter::WithEntries(arg) => super::builtin::eval_with_entries(arg, input, env),
        Filter::Walk(arg) => super::stream::walk_values(arg, input, env),
        Filter::RecurseDown { step, cond } => {
            super::stream::recurse_values(step, cond.as_deref(), input, env)
        }
        Filter::Join(arg) => eval_join(arg, input, env),
        Filter::LTrim(arg) => eval_trim(arg, input, env, true),
        Filter::RTrim(arg) => eval_trim(arg, input, env, false),
        Filter::AsciiDown => super::builtin::eval_ascii_case(input, false),
        Filter::AsciiUp => super::builtin::eval_ascii_case(input, true),
        Filter::Explode => super::builtin::eval_explode(input),
        Filter::Implode => super::builtin::eval_implode(input),
        Filter::Utf8ByteLength => super::builtin::eval_utf8_byte_length(input),
        Filter::StartsWith(arg) => super::builtin::eval_affix(arg, input, env, false),
        Filter::EndsWith(arg) => super::builtin::eval_affix(arg, input, env, true),
        Filter::IndexOf(arg) => {
            super::builtin::eval_find(arg, input, env, super::builtin::Find::Index)
        }
        Filter::RIndexOf(arg) => {
            super::builtin::eval_find(arg, input, env, super::builtin::Find::RIndex)
        }
        Filter::IndicesOf(arg) => {
            super::builtin::eval_find(arg, input, env, super::builtin::Find::Indices)
        }
        Filter::ToDate => one(date::todate(input)),
        Filter::FromDate => one(date::fromdate(input)),
        Filter::GmTime => one(date::gmtime(input)),
        Filter::LocalTime => one(date::localtime(input)),
        Filter::MkTime => one(date::mktime(input)),
        Filter::StrfTime(arg) => eval_strftime(arg, input, env),
        Filter::StrfLocalTime(arg) => eval_strflocaltime(arg, input, env),
        Filter::StrpTime(arg) => eval_strptime(arg, input, env),
        Filter::Now => Ok(vec![date::now()]),
        Filter::Math1(op) => one(apply_unary(*op, input)),
        Filter::Math2(op, left, right) => eval_math(&[left, right], input, env, |args| {
            let x = number_required(&args[0])?;
            let y = number_required(&args[1])?;
            Ok(from_f64(math::binary(*op, x, y)))
        }),
        Filter::Math3(x, y, z) => eval_math(&[x, y, z], input, env, |args| {
            let x = number_required(&args[0])?;
            let y = number_required(&args[1])?;
            let z = number_required(&args[2])?;
            Ok(from_f64(math::fma(x, y, z)))
        }),
        Filter::Abs => eval_abs(input),
        Filter::NumTest(kind) => super::builtin::eval_num_test(*kind, input),
        Filter::Halt => Err(super::builtin::stop_quiet()),
        Filter::HaltError(code) => super::builtin::eval_halt_error(code, input, env),
        Filter::Debug => super::builtin::eval_debug(input),
        Filter::Stderr => super::builtin::eval_stderr(input),
        Filter::InputLine => Ok(vec![crate::feed::input_line_value()]),
        Filter::Builtins => Ok(vec![super::builtin::builtins_value()]),
        Filter::SearchList => Ok(vec![super::modules::search_list_value()]),
        Filter::ModuleMeta => super::modules::eval_module_meta(input),
        Filter::JqOrigin => Ok(vec![Value::String(crate::feed::jq_origin())]),
        Filter::ProgOrigin => Ok(vec![Value::String(crate::feed::prog_origin())]),
        Filter::FormatBy(name) => super::builtin::eval_format_by(name, input, env),
        Filter::Pow10 => Err(raise(Value::String(
            "Error: pow10/0 not found at build time".to_string(),
        ))),
        Filter::Error => Err(raise(input.clone())),
        Filter::Raise(arg) => eval_raise(arg, input, env),
        Filter::Try { body, handler } => eval_try(body, handler.as_deref(), input, env),
        Filter::Update { op, path, expr } => {
            super::update::eval_update(*op, path, expr, input, env)
        }
        Filter::PathOf(inner) => super::update::eval_path_of(inner, input, env),
        Filter::Paths(pred) => super::update::eval_paths(pred.as_deref(), input, env),
        Filter::GetPath(arg) => super::update::eval_getpath(arg, input, env),
        Filter::SetPath(path, value) => super::update::eval_setpath(path, value, input, env),
        Filter::DelPaths(arg) => super::update::eval_delpaths(arg, input, env),
        Filter::Limit(count, expr) => super::stream::limit_values(count, expr, input, env),
        Filter::First(expr) => super::stream::first_values(expr.as_deref(), input, env),
        Filter::Last(expr) => super::stream::last_values(expr.as_deref(), input, env),
        Filter::Nth { index, expr } => {
            super::stream::nth_values(index, expr.as_deref(), input, env)
        }
        Filter::Range { from, to, step } => {
            super::stream::range_values(from.as_deref(), to, step.as_deref(), input, env)
        }
        Filter::Until(cond, next) => super::stream::until_values(cond, next, input, env),
        Filter::While(cond, update) => super::stream::while_values(cond, update, input, env),
        Filter::Repeat(expr) => super::stream::repeat_values(expr, input, env),
        Filter::Define { rest, .. } => eval_in(rest, input, env),
        Filter::Call { func, args } => super::stream::call_values(func, args, input, env),
        Filter::Param(name) => super::func::eval_param(name, input, env),
        Filter::Unresolved { name, arity, .. } => {
            Err(fail(format!("{name}/{arity} is not defined")))
        }
        Filter::MatchImpl { .. } | Filter::StreamBind { .. } => {
            super::stream::collect(filter, input, env)
        }
    }
}

pub(super) fn index_string(input: &Value, key: &str) -> Result<Vec<Value>, Failure> {
    match input {
        Value::Null => Ok(vec![Value::Null]),
        Value::Object(items) => {
            let found = items
                .iter()
                .find(|(name, _)| name == key)
                .map(|(_, value)| value.clone())
                .unwrap_or(Value::Null);
            Ok(vec![found])
        }
        other => Err(fail(format!(
            "Cannot index {} with string \"{key}\"",
            other.type_name()
        ))),
    }
}

pub(super) fn index_number(input: &Value, index: &ArrayIndex) -> Result<Vec<Value>, Failure> {
    match input {
        Value::Null => Ok(vec![Value::Null]),
        Value::Array(items) => {
            let value = match index {
                ArrayIndex::OutOfRange => Value::Null,
                ArrayIndex::At(index) => nth(items, *index),
            };
            Ok(vec![value])
        }
        other => Err(fail(format!(
            "Cannot index {} with number",
            other.type_name()
        ))),
    }
}

pub(super) fn nth(items: &[Value], index: i64) -> Value {
    let len = items.len() as i128;
    let position = if index < 0 {
        len + i128::from(index)
    } else {
        i128::from(index)
    };
    if position < 0 || position >= len {
        Value::Null
    } else {
        items[position as usize].clone()
    }
}

pub(super) fn slice(
    input: &Value,
    start: Option<&str>,
    end: Option<&str>,
) -> Result<Vec<Value>, Failure> {
    match input {
        Value::Null => Ok(vec![Value::Null]),
        Value::Array(items) => {
            let (from, to) = slice_range(start, end, items.len());
            Ok(vec![Value::array(items[from..to].to_vec())])
        }
        Value::String(text) => {
            let chars: Vec<char> = text.chars().collect();
            let (from, to) = slice_range(start, end, chars.len());
            Ok(vec![Value::String(chars[from..to].iter().collect())])
        }
        other => Err(fail(format!(
            "Cannot index {} with object",
            other.type_name()
        ))),
    }
}

/// One output per start, and for each start one output per end. The end
/// runs again for every start. A failure stops the side that produced it
/// and does not run whatever follows it.
pub(super) fn walk_slice_pairs(
    start: Option<&Filter>,
    end: Option<&Filter>,
    input: &Value,
    env: &[(String, Value)],
    mut on_pair: impl FnMut(&Value, &Value) -> Result<(), Failure>,
) -> Result<(), Failure> {
    walk_bound(start, input, env, &mut |start_value| {
        walk_bound(end, input, env, &mut |end_value| {
            on_pair(start_value, end_value)
        })
    })
}

fn walk_bound(
    bound: Option<&Filter>,
    input: &Value,
    env: &[(String, Value)],
    on_value: &mut dyn FnMut(&Value) -> Result<(), Failure>,
) -> Result<(), Failure> {
    let Some(bound) = bound else {
        return on_value(&Value::Null);
    };
    let mut stop = None;
    let result = super::stream::feed(bound, input, env, &mut |value| match on_value(value) {
        Ok(()) => false,
        Err(failure) => {
            stop = Some(failure);
            true
        }
    });
    if let Some(failure) = stop {
        return Err(failure);
    }
    result.map(|_| ())
}

fn eval_slice(
    base: Option<&Filter>,
    start: Option<&Filter>,
    end: Option<&Filter>,
    input: &Value,
    env: &[(String, Value)],
) -> Result<Vec<Value>, Failure> {
    let identity = Filter::Identity;
    let base = base.unwrap_or(&identity);
    let mut out = Vec::new();
    // Bounds are the outer stream. The base runs again for every pair, on
    // the value the brackets were applied to, not on the base's own output.
    let walked = walk_slice_pairs(start, end, input, env, |start_value, end_value| {
        let (bases, base_err) = parts(eval_in(base, input, env));
        for base_value in &bases {
            match base_value {
                Value::Null => out.push(Value::Null),
                Value::Array(_) | Value::String(_) => {
                    require_slice_index(start_value)?;
                    require_slice_index(end_value)?;
                    let start_text = number_text(start_value);
                    let end_text = number_text(end_value);
                    out.extend(slice(
                        base_value,
                        start_text.as_deref(),
                        end_text.as_deref(),
                    )?);
                }
                other => {
                    return Err(fail(format!(
                        "Cannot index {} with object",
                        other.type_name()
                    )));
                }
            }
        }
        match base_err {
            Some(failure) => Err(failure),
            None => Ok(()),
        }
    });
    match walked {
        Ok(()) => Ok(out),
        Err(failure) => Err(with_partial(out, failure)),
    }
}

pub(super) fn require_slice_index(value: &Value) -> Result<(), Failure> {
    match value {
        Value::Null | Value::Number(_) => Ok(()),
        _ => Err(fail(
            "Array/string slice indices must be integers".to_string(),
        )),
    }
}

fn number_text(value: &Value) -> Option<String> {
    match value {
        Value::Number(text) => Some(text.clone()),
        _ => None,
    }
}

pub(super) fn slice_range(start: Option<&str>, end: Option<&str>, len: usize) -> (usize, usize) {
    let len_i = i64::try_from(len).unwrap_or(i64::MAX);
    let from = match start {
        Some(literal) => slice_bound(literal, len_i, false),
        None => 0,
    };
    let to = match end {
        Some(literal) => slice_bound(literal, len_i, true),
        None => len_i,
    };
    let from = from.max(0) as usize;
    let to = to.max(0) as usize;
    let from = from.min(len);
    let to = to.min(len);
    if from >= to {
        (from, from)
    } else {
        (from, to)
    }
}

pub(super) fn recurse(input: &Value) -> Vec<Value> {
    let mut out = Vec::new();
    let mut stack = vec![input.clone()];
    while let Some(value) = stack.pop() {
        if let Ok(mut children) = iterate(&value) {
            children.reverse();
            stack.extend(children);
        }
        out.push(value);
    }
    out
}

pub(super) fn iterate(input: &Value) -> Result<Vec<Value>, Failure> {
    match input {
        Value::Array(items) => Ok(items.iter().cloned().collect()),
        Value::Object(items) => Ok(items.iter().map(|(_, value)| value.clone()).collect()),
        other => Err(fail(format!(
            "Cannot iterate over {} ({})",
            other.type_name(),
            shown(other)
        ))),
    }
}

pub(super) fn shown(value: &Value) -> String {
    clipped(value, 14)
}

/// Compact JSON, cut to `max` bytes. The last three bytes become `...`.
/// A cut through a UTF-8 sequence keeps U+FFFD in their place.
pub(super) fn clipped(value: &Value, max: usize) -> String {
    let style = Style {
        layout: Layout::Compact,
        ..Style::default()
    };
    let rendered = print::render(value, style);
    let bytes = rendered.as_bytes();
    if bytes.len() <= max {
        return rendered;
    }
    let keep = max.saturating_sub(3);
    let head = &bytes[..keep];
    let mut valid = 0;
    let mut index = 0;
    while index < head.len() {
        let width = utf8_width(head[index]);
        if width == 0 || index + width > head.len() {
            break;
        }
        if head[index + 1..index + width]
            .iter()
            .any(|byte| byte & 0xC0 != 0x80)
        {
            break;
        }
        index += width;
        valid = index;
    }
    let mut out = String::from_utf8_lossy(&head[..valid]).into_owned();
    if valid < head.len() {
        out.push('\u{FFFD}');
    }
    out.push_str("...");
    out
}

pub(super) fn utf8_width(lead: u8) -> usize {
    if lead < 0x80 {
        1
    } else if lead & 0xE0 == 0xC0 {
        2
    } else if lead & 0xF0 == 0xE0 {
        3
    } else if lead & 0xF8 == 0xF0 {
        4
    } else {
        0
    }
}

pub(super) fn fail(message: String) -> Failure {
    Failure {
        partial: Vec::new(),
        message,
        caught: None,
        stop: None,
    }
}

pub(super) fn fresh(partial: Vec<Value>, message: String) -> Failure {
    Failure {
        partial,
        message,
        caught: None,
        stop: None,
    }
}

/// `out` was produced before `failure`. The failure's own partial comes after it.
pub(super) fn extend_err(mut out: Vec<Value>, mut failure: Failure) -> Failure {
    out.extend(std::mem::take(&mut failure.partial));
    with_partial(out, failure)
}

pub(super) fn with_partial(partial: Vec<Value>, mut failure: Failure) -> Failure {
    failure.partial = partial;
    failure
}

pub(super) fn bare_fail(mut failure: Failure) -> Failure {
    failure.partial.clear();
    failure
}

pub(super) fn parts(result: Result<Vec<Value>, Failure>) -> (Vec<Value>, Option<Failure>) {
    match result {
        Ok(values) => (values, None),
        Err(mut failure) => {
            let partial = std::mem::take(&mut failure.partial);
            (partial, Some(failure))
        }
    }
}

pub(super) fn truthy(value: &Value) -> bool {
    !matches!(value, Value::Null | Value::Bool(false))
}

pub(super) fn finish(out: Vec<Value>, err: Option<Failure>) -> Result<Vec<Value>, Failure> {
    match err {
        Some(failure) => Err(with_partial(out, failure)),
        None => Ok(out),
    }
}

fn one_read(read: crate::feed::Read) -> Result<Vec<Value>, Failure> {
    match read {
        crate::feed::Read::Value(value) => Ok(vec![value]),
        crate::feed::Read::Break => Err(fail("break".to_string())),
        crate::feed::Read::Parse(message) | crate::feed::Read::Fatal(message) => Err(fail(message)),
    }
}

fn all_reads() -> Result<Vec<Value>, Failure> {
    let mut out = Vec::new();
    loop {
        match crate::feed::read_one() {
            crate::feed::Read::Value(value) => out.push(value),
            crate::feed::Read::Break => return Ok(out),
            crate::feed::Read::Parse(message) | crate::feed::Read::Fatal(message) => {
                return Err(with_partial(out, fail(message)));
            }
        }
    }
}

pub(super) fn lookup<'a>(env: &'a [(String, Value)], name: &str) -> Option<&'a Value> {
    env.iter()
        .find(|(bound, _)| bound == name)
        .map(|(_, value)| value)
}

/// New bindings are stored first, so the earliest one in the frame wins.
pub(super) fn child_scope(
    frame: &[(String, Value)],
    env: &[(String, Value)],
) -> Vec<(String, Value)> {
    let mut child = Vec::with_capacity(frame.len() + env.len());
    child.extend_from_slice(frame);
    child.extend_from_slice(env);
    child
}

pub(super) fn eval_bind(
    expr: &Filter,
    pattern: &Pattern,
    alts: &[Pattern],
    body: &Filter,
    input: &Value,
    env: &[(String, Value)],
) -> Result<Vec<Value>, Failure> {
    let names = bind_name_list(pattern, alts);
    let (values, err) = parts(eval_in(expr, input, env));
    let mut out = Vec::new();
    for value in values {
        if let Err(failure) = bind_value(pattern, alts, &names, &value, body, input, env, &mut out)
        {
            return Err(with_partial(out, failure));
        }
    }
    finish(out, err)
}

/// One value against each `?//` pattern. A pattern with no match is success
/// and does not try the next pattern. A failure tries the next one. `halt`
/// does not. Outputs from a failed attempt stay.
fn bind_value(
    pattern: &Pattern,
    alts: &[Pattern],
    names: &[String],
    value: &Value,
    body: &Filter,
    input: &Value,
    env: &[(String, Value)],
    out: &mut Vec<Value>,
) -> Result<(), Failure> {
    let mut pats = Vec::with_capacity(1 + alts.len());
    pats.push(pattern);
    pats.extend(alts);
    for (index, pat) in pats.iter().enumerate() {
        let last = index + 1 == pats.len();
        match bind_pattern(pat, names, value, body, input, env, out) {
            Ok(()) => return Ok(()),
            Err(failure) if failure.stop.is_some() || last => return Err(failure),
            Err(failure) => out.extend(failure.partial),
        }
    }
    Ok(())
}

fn bind_pattern(
    pattern: &Pattern,
    names: &[String],
    value: &Value,
    body: &Filter,
    input: &Value,
    env: &[(String, Value)],
    out: &mut Vec<Value>,
) -> Result<(), Failure> {
    match_pat(pattern, value, env, &[], &mut |frame| {
        let frame = with_nulls(frame, names);
        let child = child_scope(&frame, env);
        match eval_in(body, input, &child) {
            Ok(more) => {
                out.extend(more);
                Ok(())
            }
            Err(mut failure) => {
                out.extend(std::mem::take(&mut failure.partial));
                Err(failure)
            }
        }
    })
}

fn bind_name_list(pattern: &Pattern, alts: &[Pattern]) -> Vec<String> {
    let mut names = Vec::new();
    super::parse::bind_names(pattern, alts, &mut names);
    names
}

/// Names the successful pattern did not bind are null, and they hide an outer
/// binding of the same name.
fn with_nulls(frame: &[(String, Value)], names: &[String]) -> Vec<(String, Value)> {
    let mut out = frame.to_vec();
    for name in names {
        if !out.iter().any(|(bound, _)| bound == name) {
            out.push((name.clone(), Value::Null));
        }
    }
    out
}

struct LabelMark {
    slot: u32,
    runtime: u32,
}

thread_local! {
    static LABEL_STACK: RefCell<Vec<LabelMark>> = const { RefCell::new(Vec::new()) };
    static LABEL_SEQ: Cell<u32> = const { Cell::new(0) };
}

/// A new run starts at 0. Inputs in that run keep counting.
pub(crate) fn reset_labels() {
    LABEL_SEQ.with(|seq| seq.set(0));
    LABEL_STACK.with(|stack| stack.borrow_mut().clear());
}

/// Pops this label when dropped, including when a caller stops the stack early.
pub(super) struct LabelGuard {
    runtime: u32,
}

impl LabelGuard {
    pub(super) fn enter(slot: u32) -> (u32, Self) {
        let runtime = LABEL_SEQ.with(|seq| {
            let id = seq.get();
            seq.set(id.wrapping_add(1));
            id
        });
        LABEL_STACK.with(|stack| {
            stack.borrow_mut().push(LabelMark { slot, runtime });
        });
        (runtime, LabelGuard { runtime })
    }
}

impl Drop for LabelGuard {
    fn drop(&mut self) {
        LABEL_STACK.with(|stack| {
            let mut stack = stack.borrow_mut();
            if let Some(index) = stack.iter().rposition(|mark| mark.runtime == self.runtime) {
                stack.remove(index);
            }
        });
    }
}

pub(super) fn lookup_label(slot: u32) -> Option<u32> {
    LABEL_STACK.with(|stack| {
        stack
            .borrow()
            .iter()
            .rev()
            .find(|mark| mark.slot == slot)
            .map(|mark| mark.runtime)
    })
}

pub(super) fn label_object(runtime: u32) -> Value {
    Value::object(vec![(
        "__jq".to_string(),
        Value::Number(runtime.to_string()),
    )])
}

/// A break, or `error({"__jq": n})` for this label's number.
pub(super) fn is_label_hit(failure: &Failure, runtime: u32) -> bool {
    if failure.stop.is_some() {
        return false;
    }
    match &failure.caught {
        Some(value) => label_runtime(value) == Some(runtime),
        None => false,
    }
}

fn label_runtime(value: &Value) -> Option<u32> {
    let Value::Object(items) = value else {
        return None;
    };
    if items.len() != 1 {
        return None;
    }
    let (key, inner) = &items[0];
    if key != "__jq" {
        return None;
    }
    let Value::Number(text) = inner else {
        return None;
    };
    let number = crate::number::f64_of(text);
    if number.is_finite() && number >= 0.0 && number.fract() == 0.0 && number <= u32::MAX as f64 {
        Some(number as u32)
    } else {
        None
    }
}

pub(super) fn break_failure(slot: Option<u32>) -> Failure {
    let Some(runtime) = slot.and_then(lookup_label) else {
        return fail("break".to_string());
    };
    super::builtin::raise(label_object(runtime))
}

fn eval_label(
    slot: u32,
    body: &Filter,
    input: &Value,
    env: &[(String, Value)],
) -> Result<Vec<Value>, Failure> {
    let (runtime, guard) = LabelGuard::enter(slot);
    let result = eval_in(body, input, env);
    drop(guard);
    match result {
        Ok(values) => Ok(values),
        Err(failure) if is_label_hit(&failure, runtime) => Ok(failure.partial),
        Err(failure) => Err(failure),
    }
}

/// `reduce` keeps only a finished accumulator. `foreach` emits each step.
/// The stream of every init after the first is run on null.
pub(super) fn eval_fold(
    stream: &Filter,
    pattern: &Pattern,
    alts: &[Pattern],
    init: &Filter,
    update: &Filter,
    extract: Option<&Filter>,
    emit: bool,
    input: &Value,
    env: &[(String, Value)],
) -> Result<Vec<Value>, Failure> {
    let mut out = Vec::new();
    let mut sink = |value: &Value| {
        out.push(value.clone());
        false
    };
    match fold_into(
        stream, pattern, alts, init, update, extract, emit, input, env, &mut sink,
    ) {
        Ok(_) => Ok(out),
        Err(failure) => Err(with_partial(out, failure)),
    }
}

/// One `reduce` or `foreach` step. `true` means the caller asked to stop.
fn fold_one(
    item: &Value,
    pattern: &Pattern,
    alts: &[Pattern],
    update: &Filter,
    extract: Option<&Filter>,
    emit: bool,
    env: &[(String, Value)],
    acc: &mut Value,
    lookup: &mut Option<super::update::ObjIndex>,
    sink: &mut dyn FnMut(&Value) -> bool,
) -> Result<bool, Failure> {
    let names = bind_name_list(pattern, alts);
    let mut pats = Vec::with_capacity(1 + alts.len());
    pats.push(pattern);
    pats.extend(alts);
    for (index, pat) in pats.iter().enumerate() {
        let last = index + 1 == pats.len();
        match fold_pattern(
            item, pat, &names, update, extract, emit, env, acc, lookup, sink,
        ) {
            Ok(stop) => return Ok(stop),
            Err(failure) if failure.stop.is_some() || last => return Err(failure),
            Err(_) => {}
        }
    }
    Ok(false)
}

fn fold_pattern(
    item: &Value,
    pattern: &Pattern,
    names: &[String],
    update: &Filter,
    extract: Option<&Filter>,
    emit: bool,
    env: &[(String, Value)],
    acc: &mut Value,
    lookup: &mut Option<super::update::ObjIndex>,
    sink: &mut dyn FnMut(&Value) -> bool,
) -> Result<bool, Failure> {
    let mut stop = false;
    let stepped = match_pat(pattern, item, env, &[], &mut |frame| {
        let frame = with_nulls(frame, names);
        let frame = frame.as_slice();
        if stop {
            return Ok(());
        }
        let child = child_scope(frame, env);
        // `foreach` can reuse the accumulator when the step it emits
        // is that accumulator. The caller has to be finished with the
        // value before the next step.
        let accelerated = if !emit || super::update::foreach_can_accelerate(update) {
            match super::update::accelerate_reduce(update, acc, &child, lookup) {
                Ok(true) => true,
                Ok(false) => {
                    *lookup = None;
                    false
                }
                Err(failure) => return Err(failure),
            }
        } else {
            false
        };
        if accelerated {
            if emit && deliver(acc, extract, &child, sink)? {
                stop = true;
            }
            return Ok(());
        }
        let (updates, update_err) = parts(eval_in(update, acc, &child));
        if emit {
            if let Some(extract) = extract {
                for value in &updates {
                    match eval_in(extract, value, &child) {
                        Ok(more) => {
                            for one in &more {
                                if sink(one) {
                                    stop = true;
                                    return Ok(());
                                }
                            }
                        }
                        Err(mut failure) => {
                            for one in &failure.partial {
                                if sink(one) {
                                    stop = true;
                                    return Ok(());
                                }
                            }
                            failure.partial.clear();
                            return Err(failure);
                        }
                    }
                }
            } else {
                for value in &updates {
                    if sink(value) {
                        stop = true;
                        return Ok(());
                    }
                }
            }
        }
        if let Some(failure) = update_err {
            return Err(failure);
        }
        *acc = match updates.last() {
            Some(value) => value.clone(),
            None => Value::Null,
        };
        *lookup = None;
        Ok(())
    });
    if let Err(failure) = stepped {
        return Err(failure);
    }
    Ok(stop)
}

/// `reduce` once the initial value and the stream values are already known.
pub(super) fn reduce_finished(
    init: &Value,
    items: &[Value],
    pattern: &Pattern,
    alts: &[Pattern],
    update: &Filter,
    env: &[(String, Value)],
) -> Result<Value, Failure> {
    let mut acc = init.clone();
    let mut lookup = None;
    for item in items {
        fold_one(
            item,
            pattern,
            alts,
            update,
            None,
            false,
            env,
            &mut acc,
            &mut lookup,
            &mut |_| false,
        )?;
    }
    Ok(acc)
}

/// `true` means a consumer asked to stop. Values already handed to `sink`
/// are not repeated in an error. `sink` returns `true` to stop.
pub(super) fn fold_into(
    stream: &Filter,
    pattern: &Pattern,
    alts: &[Pattern],
    init: &Filter,
    update: &Filter,
    extract: Option<&Filter>,
    emit: bool,
    input: &Value,
    env: &[(String, Value)],
    sink: &mut dyn FnMut(&Value) -> bool,
) -> Result<bool, Failure> {
    let (inits, init_err) = parts(eval_in(init, input, env));
    if inits.is_empty() {
        return match init_err {
            Some(failure) => Err(failure),
            None => Ok(false),
        };
    }
    for (index, init_value) in inits.iter().enumerate() {
        let null_input = Value::Null;
        let stream_input = if index == 0 { input } else { &null_input };
        let mut acc = init_value.clone();
        // Reused while the accumulator stays one object. A slow step replaces
        // the value, and the next fast step builds the index again.
        let mut lookup = None;
        let mut stream_err = None;
        let mut stop = false;
        // `inputs` is read one value at a time. The update sees that value's
        // line, and the read past the end runs only after the update.
        if matches!(stream, Filter::Inputs) {
            while !stop {
                match crate::feed::read_one() {
                    crate::feed::Read::Value(item) => {
                        if fold_one(
                            &item,
                            pattern,
                            alts,
                            update,
                            extract,
                            emit,
                            env,
                            &mut acc,
                            &mut lookup,
                            sink,
                        )? {
                            stop = true;
                        }
                    }
                    crate::feed::Read::Break => break,
                    crate::feed::Read::Parse(message) | crate::feed::Read::Fatal(message) => {
                        stream_err = Some(fail(message));
                        break;
                    }
                }
            }
        } else {
            let (items, err) = parts(eval_in(stream, stream_input, env));
            stream_err = err;
            for item in &items {
                if fold_one(
                    item,
                    pattern,
                    alts,
                    update,
                    extract,
                    emit,
                    env,
                    &mut acc,
                    &mut lookup,
                    sink,
                )? {
                    stop = true;
                    break;
                }
            }
        }
        if stop {
            return Ok(true);
        }
        if let Some(failure) = stream_err {
            return Err(failure);
        }
        if !emit && sink(&acc) {
            return Ok(true);
        }
    }
    match init_err {
        Some(failure) => Err(failure),
        None => Ok(false),
    }
}

/// Emit one accelerated `foreach` step. `true` means stop.
fn deliver(
    acc: &Value,
    extract: Option<&Filter>,
    env: &[(String, Value)],
    sink: &mut dyn FnMut(&Value) -> bool,
) -> Result<bool, Failure> {
    // No extract, or `.`, emits the accumulator itself. Borrowing it keeps
    // the next step from copying the array.
    if extract.is_none_or(|extract| matches!(extract, Filter::Identity)) {
        return Ok(sink(acc));
    }
    let extract = extract.expect("extract");
    match eval_in(extract, acc, env) {
        Ok(more) => {
            for one in &more {
                if sink(one) {
                    return Ok(true);
                }
            }
            Ok(false)
        }
        Err(mut failure) => {
            for one in &failure.partial {
                if sink(one) {
                    return Ok(true);
                }
            }
            failure.partial.clear();
            Err(failure)
        }
    }
}

/// A pattern is indexing. Array elements are stored right to left, object
/// fields left to right, and an `also` name before its subpattern. The first
/// stored binding of a name wins. The body runs inside the cross product.
pub(super) fn match_pat(
    pattern: &Pattern,
    value: &Value,
    env: &[(String, Value)],
    binds: &[(String, Value)],
    cont: &mut dyn FnMut(&[(String, Value)]) -> Result<(), Failure>,
) -> Result<(), Failure> {
    match pattern {
        Pattern::Var(name) => {
            let mut next = Vec::with_capacity(binds.len() + 1);
            next.extend_from_slice(binds);
            next.push((name.clone(), value.clone()));
            cont(&next)
        }
        Pattern::Array(pats) => match_array(pats, value, env, binds, pats.len(), cont),
        Pattern::Object(fields) => match_object(fields, value, env, binds, 0, cont),
    }
}

pub(super) fn match_array(
    pats: &[Pattern],
    value: &Value,
    env: &[(String, Value)],
    binds: &[(String, Value)],
    index: usize,
    cont: &mut dyn FnMut(&[(String, Value)]) -> Result<(), Failure>,
) -> Result<(), Failure> {
    if index == 0 {
        return cont(binds);
    }
    let element = match index_number(value, &ArrayIndex::At((index - 1) as i64)) {
        Ok(got) => got.into_iter().next().unwrap_or(Value::Null),
        Err(failure) => return Err(failure),
    };
    let sub = pats[index - 1].clone();
    match_pat(&sub, &element, env, binds, &mut |inner| {
        match_array(pats, value, env, inner, index - 1, cont)
    })
}

pub(super) fn match_object(
    fields: &[PatField],
    value: &Value,
    env: &[(String, Value)],
    binds: &[(String, Value)],
    index: usize,
    cont: &mut dyn FnMut(&[(String, Value)]) -> Result<(), Failure>,
) -> Result<(), Failure> {
    let Some(field) = fields.get(index) else {
        return cont(binds);
    };
    let field = field.clone();
    let (keys, key_err) = parts(eval_in(&field.key, value, env));
    for key in keys {
        let elements = match index_by(value, &key) {
            Ok(got) => got,
            Err(failure) => return Err(failure),
        };
        for element in elements {
            let mut next = Vec::with_capacity(binds.len() + 1);
            next.extend_from_slice(binds);
            if let Some(name) = &field.also {
                next.push((name.clone(), element.clone()));
            }
            match_pat(&field.pattern, &element, env, &next, &mut |inner| {
                match_object(fields, value, env, inner, index + 1, cont)
            })?;
        }
    }
    if let Some(failure) = key_err {
        return Err(failure);
    }
    Ok(())
}

pub(super) fn eval_neg(
    inner: &Filter,
    input: &Value,
    env: &[(String, Value)],
) -> Result<Vec<Value>, Failure> {
    let (values, err) = parts(eval_in(inner, input, env));
    let mut out = Vec::new();
    for value in values {
        match negate_value(&value) {
            Ok(next) => out.push(next),
            Err(message) => return Err(fresh(out, message)),
        }
    }
    finish(out, err)
}

pub(super) fn eval_bin(
    op: Op,
    left: &Filter,
    right: &Filter,
    input: &Value,
    env: &[(String, Value)],
) -> Result<Vec<Value>, Failure> {
    // The right stream is the outer loop. A left error drops the remaining
    // rights; a right error is reported after the successful rights.
    let (rights, right_err) = parts(eval_in(right, input, env));
    let mut out = Vec::new();
    for right_value in rights {
        let (lefts, left_err) = parts(eval_in(left, input, env));
        for left_value in lefts {
            match apply_op(op, &left_value, &right_value) {
                Ok(value) => out.push(value),
                Err(message) => return Err(fresh(out, message)),
            }
        }
        if let Some(failure) = left_err {
            return Err(with_partial(out, failure));
        }
    }
    finish(out, right_err)
}

pub(super) fn eval_and(
    left: &Filter,
    right: &Filter,
    input: &Value,
    env: &[(String, Value)],
) -> Result<Vec<Value>, Failure> {
    let (lefts, left_err) = parts(eval_in(left, input, env));
    let mut out = Vec::new();
    for left_value in lefts {
        if !truthy(&left_value) {
            out.push(Value::Bool(false));
            continue;
        }
        let (rights, right_err) = parts(eval_in(right, input, env));
        for right_value in rights {
            out.push(Value::Bool(truthy(&right_value)));
        }
        if let Some(failure) = right_err {
            return Err(with_partial(out, failure));
        }
    }
    finish(out, left_err)
}

pub(super) fn eval_or(
    left: &Filter,
    right: &Filter,
    input: &Value,
    env: &[(String, Value)],
) -> Result<Vec<Value>, Failure> {
    let (lefts, left_err) = parts(eval_in(left, input, env));
    let mut out = Vec::new();
    for left_value in lefts {
        if truthy(&left_value) {
            out.push(Value::Bool(true));
            continue;
        }
        let (rights, right_err) = parts(eval_in(right, input, env));
        for right_value in rights {
            out.push(Value::Bool(truthy(&right_value)));
        }
        if let Some(failure) = right_err {
            return Err(with_partial(out, failure));
        }
    }
    finish(out, left_err)
}

pub(super) fn eval_alt(
    left: &Filter,
    right: &Filter,
    input: &Value,
    env: &[(String, Value)],
) -> Result<Vec<Value>, Failure> {
    match eval_in(left, input, env) {
        Err(mut failure) => {
            let kept = std::mem::take(&mut failure.partial)
                .into_iter()
                .filter(truthy)
                .collect();
            Err(with_partial(kept, failure))
        }
        Ok(values) => {
            let defined: Vec<Value> = values.into_iter().filter(truthy).collect();
            if defined.is_empty() {
                eval_in(right, input, env)
            } else {
                Ok(defined)
            }
        }
    }
}

pub(super) fn eval_if(
    cond: &Filter,
    yes: &Filter,
    no: &Filter,
    input: &Value,
    env: &[(String, Value)],
) -> Result<Vec<Value>, Failure> {
    let (conds, err) = parts(eval_in(cond, input, env));
    let mut out = Vec::new();
    for cond_value in conds {
        let branch = if truthy(&cond_value) { yes } else { no };
        match eval_in(branch, input, env) {
            Ok(values) => out.extend(values),
            Err(failure) => return Err(extend_err(out, failure)),
        }
    }
    finish(out, err)
}

pub(super) fn eval_interpolate(
    pieces: &[Piece],
    input: &Value,
    env: &[(String, Value)],
) -> Result<Vec<Value>, Failure> {
    let strings = expand_pieces(pieces, 0, String::new(), input, env)?;
    Ok(strings.into_iter().map(Value::String).collect())
}

pub(super) fn expand_pieces(
    pieces: &[Piece],
    index: usize,
    prefix: String,
    input: &Value,
    env: &[(String, Value)],
) -> Result<Vec<String>, Failure> {
    let Some(piece) = pieces.get(index) else {
        return Ok(vec![prefix]);
    };
    match piece {
        Piece::Text(text) => {
            let mut next = prefix;
            next.push_str(text);
            expand_pieces(pieces, index + 1, next, input, env)
        }
        Piece::Interp(filter) => match eval_in(filter, input, env) {
            Ok(values) => {
                let mut out = Vec::new();
                for value in values {
                    let mut next = prefix.clone();
                    next.push_str(&jq_text(&value));
                    out.extend(expand_pieces(pieces, index + 1, next, input, env)?);
                }
                Ok(out)
            }
            Err(failure) => {
                let caught = failure.caught.clone();
                let message = failure.message.clone();
                let mut partial = Vec::new();
                for value in failure.partial {
                    let mut next = prefix.clone();
                    next.push_str(&jq_text(&value));
                    match expand_pieces(pieces, index + 1, next, input, env) {
                        Ok(more) => partial.extend(more),
                        Err(inner) => {
                            for value in inner.partial {
                                if let Value::String(text) = &value {
                                    partial.push(text.clone());
                                }
                            }
                            // An inner piece that fails reuses this piece's error.
                            return Err(Failure {
                                partial: partial.into_iter().map(Value::String).collect(),
                                message,
                                caught,
                                stop: failure.stop.clone(),
                            });
                        }
                    }
                }
                Err(Failure {
                    partial: partial.into_iter().map(Value::String).collect(),
                    message,
                    caught,
                    stop: failure.stop,
                })
            }
        },
    }
}

pub(super) fn jq_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => {
            let style = Style {
                layout: Layout::Compact,
                ..Style::default()
            };
            print::render(other, style)
        }
    }
}

pub(super) fn eval_object(
    fields: &[ObjField],
    input: &Value,
    env: &[(String, Value)],
) -> Result<Vec<Value>, Failure> {
    let mut streams = Vec::with_capacity(fields.len());
    for field in fields {
        streams.push(field_alts(field, input, env));
    }
    combine_fields(&streams, 0, Vec::new())
}

pub(super) struct Alts {
    pub(super) pairs: Vec<(String, Value)>,
    pub(super) error: Option<Failure>,
}

pub(super) fn field_alts(field: &ObjField, input: &Value, env: &[(String, Value)]) -> Alts {
    match field {
        ObjField::Lookup(key) => lookup_alts(key, input, env),
        ObjField::Pair { key, value } => pair_alts(key, value, input, env),
    }
}

pub(super) fn lookup_alts(key: &Filter, input: &Value, env: &[(String, Value)]) -> Alts {
    let (keys, key_err) = parts(eval_in(key, input, env));
    let mut pairs = Vec::new();
    for value in keys {
        let Ok(name) = value.clone().into_string() else {
            return Alts {
                pairs,
                error: Some(fail(key_message(&value))),
            };
        };
        match index_by(input, &Value::String(name.clone())) {
            Ok(got) => {
                let one = got.into_iter().next().unwrap_or(Value::Null);
                pairs.push((name, one));
            }
            Err(failure) => {
                return Alts {
                    pairs,
                    error: Some(bare_fail(failure)),
                };
            }
        }
    }
    Alts {
        pairs,
        error: key_err,
    }
}

pub(super) fn pair_alts(
    key: &Filter,
    value: &Filter,
    input: &Value,
    env: &[(String, Value)],
) -> Alts {
    let (keys, key_err) = parts(eval_in(key, input, env));
    let (values, value_err) = parts(eval_in(value, input, env));
    let mut pairs = Vec::new();
    for key_value in &keys {
        let Value::String(name) = key_value else {
            return Alts {
                pairs,
                error: Some(fail(key_message(key_value))),
            };
        };
        for item in &values {
            pairs.push((name.clone(), item.clone()));
        }
        if let Some(failure) = &value_err {
            return Alts {
                pairs,
                error: Some(failure.clone()),
            };
        }
    }
    Alts {
        pairs,
        error: key_err.or(value_err),
    }
}

pub(super) fn combine_fields(
    streams: &[Alts],
    index: usize,
    base: Vec<(String, Value)>,
) -> Result<Vec<Value>, Failure> {
    let Some(stream) = streams.get(index) else {
        return Ok(vec![Value::object(base)]);
    };
    let mut out = Vec::new();
    for (key, value) in &stream.pairs {
        let mut next = base.clone();
        put(&mut next, key.clone(), value.clone());
        match combine_fields(streams, index + 1, next) {
            Ok(more) => out.extend(more),
            Err(failure) => return Err(extend_err(out, failure)),
        }
    }
    if let Some(failure) = &stream.error {
        return Err(with_partial(out, failure.clone()));
    }
    Ok(out)
}

pub(super) fn eval_at(
    base: &Filter,
    key: &Filter,
    optional: bool,
    input: &Value,
    env: &[(String, Value)],
) -> Result<Vec<Value>, Failure> {
    let values = eval_in(base, input, env)?;
    let keys = match eval_in(key, input, env) {
        Ok(keys) => keys,
        Err(failure) => {
            let caught = failure.caught.clone();
            let message = failure.message;
            let mut partial = Vec::new();
            for key in &failure.partial {
                for value in &values {
                    match index_by(value, key) {
                        Ok(got) => partial.extend(got),
                        Err(inner) => {
                            partial.extend(inner.partial);
                            // The key's error wins over an index error on a partial key.
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
    };
    let mut out = Vec::new();
    for key in &keys {
        for value in &values {
            match index_by(value, key) {
                Ok(got) => out.extend(got),
                Err(mut failure) => {
                    out.extend(std::mem::take(&mut failure.partial));
                    if failure.stop.is_some() || !optional {
                        return Err(with_partial(out, failure));
                    }
                }
            }
        }
    }
    Ok(out)
}

pub(super) fn index_by(base: &Value, key: &Value) -> Result<Vec<Value>, Failure> {
    match key {
        Value::String(name) => index_string(base, name),
        Value::Number(text) => {
            let index = match array_index(text) {
                Some(index) => ArrayIndex::At(index),
                None => ArrayIndex::OutOfRange,
            };
            index_number(base, &index)
        }
        Value::Object(_) if matches!(base, Value::Null) => Ok(vec![Value::Null]),
        Value::Object(_) if matches!(base, Value::Array(_) | Value::String(_)) => Err(fail(
            "Array/string slice indices must be integers".to_string(),
        )),
        Value::Array(_) if matches!(base, Value::Array(_)) => Ok(vec![Value::array(Vec::new())]),
        other => Err(fail(format!(
            "Cannot index {} with {}",
            base.type_name(),
            other.type_name()
        ))),
    }
}
