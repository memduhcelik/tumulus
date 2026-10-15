//! Builtins: length, keys, type, strings, math, dates, try, and error.

use crate::date;
use crate::math;
use crate::number;
use crate::value::Value;

use super::ast::*;
use super::eval::*;
use super::value::*;

/// `error` and `error(expr)` record the value. The message is what an uncaught
/// string prints, and what `catch` receives when no value was recorded.
pub(super) fn raise(value: Value) -> Failure {
    let message = match &value {
        Value::String(text) => text.clone(),
        other => jq_text(other),
    };
    Failure {
        partial: Vec::new(),
        message,
        caught: Some(value),
    }
}

pub(super) fn eval_raise(
    arg: &Filter,
    input: &Value,
    env: &[(String, Value)],
) -> Result<Vec<Value>, Failure> {
    // The first value is raised. Later values are not. An argument that
    // produces nothing and then fails propagates that failure.
    let (values, err) = parts(eval_in(arg, input, env));
    if let Some(value) = values.into_iter().next() {
        return Err(raise(value));
    }
    match err {
        Some(failure) => Err(failure),
        None => Ok(Vec::new()),
    }
}

pub(super) fn eval_try(
    body: &Filter,
    handler: Option<&Filter>,
    input: &Value,
    env: &[(String, Value)],
) -> Result<Vec<Value>, Failure> {
    let failure = match eval_in(body, input, env) {
        Ok(values) => return Ok(values),
        Err(failure) => failure,
    };
    let Some(handler) = handler else {
        return Ok(failure.partial);
    };
    let caught = match failure.caught {
        Some(value) => value,
        None => Value::String(failure.message),
    };
    match eval_in(handler, &caught, env) {
        Ok(more) => {
            let mut out = failure.partial;
            out.extend(more);
            Ok(out)
        }
        Err(handler_err) => Err(extend_err(failure.partial, handler_err)),
    }
}

pub(super) fn one(result: Result<Value, String>) -> Result<Vec<Value>, Failure> {
    result.map(|value| vec![value]).map_err(fail)
}

pub(super) fn number_required(value: &Value) -> Result<f64, String> {
    match value {
        Value::Number(text) => Ok(number::f64_of(text)),
        other => Err(format!(
            "{} ({}) number required",
            other.type_name(),
            shown(other)
        )),
    }
}

pub(super) fn from_f64(n: f64) -> Value {
    Value::Number(number::mark_f64(number::format_jq_f64(n)))
}

pub(super) fn eval_abs(input: &Value) -> Result<Vec<Value>, Failure> {
    let zero = Value::Number("0".to_string());
    if cmp_value(input, &zero) < 0 {
        one(negate_value(input))
    } else {
        Ok(vec![input.clone()])
    }
}

pub(super) fn apply_unary(op: math::Unary, input: &Value) -> Result<Value, String> {
    let n = number_required(input)?;
    Ok(match math::unary(op, n) {
        math::Out::Num(n) => from_f64(n),
        math::Out::Pair(first, second) => Value::Array(vec![from_f64(first), from_f64(second)]),
    })
}

/// The rightmost argument is the outer loop. An argument is run again for
/// every value of the arguments to its right. The first failure stops the
/// rest, and values already produced stay.
pub(super) fn eval_math(
    args: &[&Filter],
    input: &Value,
    env: &[(String, Value)],
    apply: impl Fn(&[Value]) -> Result<Value, String>,
) -> Result<Vec<Value>, Failure> {
    let mut out = Vec::new();
    let mut acc = vec![Value::Null; args.len()];
    fill_math(args, args.len(), &mut acc, input, env, &apply, &mut out)?;
    Ok(out)
}

pub(super) fn fill_math(
    args: &[&Filter],
    index: usize,
    acc: &mut [Value],
    input: &Value,
    env: &[(String, Value)],
    apply: &impl Fn(&[Value]) -> Result<Value, String>,
    out: &mut Vec<Value>,
) -> Result<(), Failure> {
    if index == 0 {
        return match apply(acc) {
            Ok(value) => {
                out.push(value);
                Ok(())
            }
            Err(message) => Err(fresh(std::mem::take(out), message)),
        };
    }
    let slot = index - 1;
    let (values, err) = parts(eval_in(args[slot], input, env));
    for value in values {
        acc[slot] = value;
        fill_math(args, slot, acc, input, env, apply, out)?;
    }
    match err {
        Some(failure) => Err(with_partial(std::mem::take(out), failure)),
        None => Ok(()),
    }
}

pub(super) fn eval_strftime(
    arg: &Filter,
    input: &Value,
    env: &[(String, Value)],
) -> Result<Vec<Value>, Failure> {
    eval_format(arg, input, env, date::format_time)
}

pub(super) fn eval_strflocaltime(
    arg: &Filter,
    input: &Value,
    env: &[(String, Value)],
) -> Result<Vec<Value>, Failure> {
    eval_format(arg, input, env, date::strflocaltime)
}

pub(super) fn eval_format(
    arg: &Filter,
    input: &Value,
    env: &[(String, Value)],
    format: fn(&Value, &Value) -> Result<Value, String>,
) -> Result<Vec<Value>, Failure> {
    let (formats, err) = parts(eval_in(arg, input, env));
    let mut out = Vec::new();
    for spec in formats {
        match format(input, &spec) {
            Ok(value) => out.push(value),
            Err(message) => return Err(fresh(out, message)),
        }
    }
    finish(out, err)
}

pub(super) fn eval_strptime(
    arg: &Filter,
    input: &Value,
    env: &[(String, Value)],
) -> Result<Vec<Value>, Failure> {
    let (formats, err) = parts(eval_in(arg, input, env));
    let mut out = Vec::new();
    for format in formats {
        match date::strptime_fmt(input, &format) {
            Ok(value) => out.push(value),
            Err(message) => return Err(fresh(out, message)),
        }
    }
    finish(out, err)
}

pub(super) fn eval_length(input: &Value) -> Result<Vec<Value>, Failure> {
    match input {
        Value::Null => Ok(vec![Value::Number("0".to_string())]),
        Value::String(text) => Ok(vec![Value::Number(text.chars().count().to_string())]),
        Value::Array(items) => Ok(vec![Value::Number(items.len().to_string())]),
        Value::Object(items) => Ok(vec![Value::Number(items.len().to_string())]),
        Value::Number(text) => Ok(vec![number_length(text)]),
        other => Err(fail(format!(
            "{} ({}) has no length",
            other.type_name(),
            shown(other)
        ))),
    }
}

/// The absolute value is a computed number. A preserved literal and its
/// length can therefore print differently.
pub(super) fn number_length(text: &str) -> Value {
    let abs = match number::real(text) {
        number::Real::Nan => number::nan_text(),
        number::Real::Num(n) => number::format_jq_f64(n.abs()),
    };
    Value::Number(number::mark_f64(abs))
}

pub(super) fn eval_keys(input: &Value) -> Result<Vec<Value>, Failure> {
    match input {
        Value::Array(items) => {
            let keys = (0..items.len())
                .map(|index| Value::Number(index.to_string()))
                .collect();
            Ok(vec![Value::Array(keys)])
        }
        Value::Object(items) => {
            let mut names: Vec<&str> = items.iter().map(|(name, _)| name.as_str()).collect();
            names.sort_unstable();
            names.dedup();
            let keys = names
                .into_iter()
                .map(|name| Value::String(name.to_string()))
                .collect();
            Ok(vec![Value::Array(keys)])
        }
        other => Err(fail(format!(
            "{} ({}) has no keys",
            other.type_name(),
            shown(other)
        ))),
    }
}

pub(super) fn eval_has(
    arg: &Filter,
    input: &Value,
    env: &[(String, Value)],
) -> Result<Vec<Value>, Failure> {
    let (keys, err) = parts(eval_in(arg, input, env));
    let mut out = Vec::new();
    for key in keys {
        match has_key(input, &key) {
            Ok(value) => out.push(value),
            Err(message) => return Err(fresh(out, message)),
        }
    }
    finish(out, err)
}

/// Null has no keys, and asking is false for every key. An array index is
/// truncated toward zero and does not wrap; `-0.1` truncates to `-0`, which
/// is present in a non-empty array.
pub(super) fn has_key(input: &Value, key: &Value) -> Result<Value, String> {
    if matches!(input, Value::Null) {
        return Ok(Value::Bool(false));
    }
    match (input, key) {
        (Value::Object(items), Value::String(name)) => {
            Ok(Value::Bool(items.iter().any(|(key, _)| key == name)))
        }
        (Value::Array(items), Value::Number(text)) => Ok(Value::Bool(array_has(items.len(), text))),
        _ => Err(format!(
            "Cannot check whether {} has a {} key",
            input.type_name(),
            key.type_name()
        )),
    }
}

pub(super) fn array_has(len: usize, text: &str) -> bool {
    let number::Real::Num(n) = number::real(text) else {
        return false;
    };
    if !n.is_finite() {
        return false;
    }
    let index = n.trunc();
    index >= 0.0 && index < len as f64
}

pub(super) fn eval_select(
    arg: &Filter,
    input: &Value,
    env: &[(String, Value)],
) -> Result<Vec<Value>, Failure> {
    let (values, err) = parts(eval_in(arg, input, env));
    let mut out = Vec::new();
    for value in values {
        if truthy(&value) {
            out.push(input.clone());
        }
    }
    finish(out, err)
}

/// Compact JSON. A string is quoted. Anything else matches `tostring`.
pub(super) fn eval_tojson(input: &Value) -> Result<Vec<Value>, Failure> {
    let style = crate::print::Style {
        layout: crate::print::Layout::Compact,
        ..crate::print::Style::default()
    };
    Ok(vec![Value::String(crate::print::render(input, style))])
}

/// `reduce .[] as $x (null; . + $x)`. An empty walk is null.
pub(super) fn eval_add(input: &Value) -> Result<Vec<Value>, Failure> {
    let items = iterate(input)?;
    let mut acc = Value::Null;
    for item in items {
        acc = add_values(&acc, &item).map_err(fail)?;
    }
    Ok(vec![acc])
}

pub(super) fn eval_to_entries(input: &Value) -> Result<Vec<Value>, Failure> {
    let entries = match input {
        Value::Array(items) => items
            .iter()
            .enumerate()
            .map(|(index, value)| entry(Value::Number(index.to_string()), value.clone()))
            .collect(),
        Value::Object(pairs) => pairs
            .iter()
            .map(|(key, value)| entry(Value::String(key.clone()), value.clone()))
            .collect(),
        other => {
            return Err(fail(format!(
                "{} ({}) has no keys",
                other.type_name(),
                shown(other)
            )));
        }
    };
    Ok(vec![Value::Array(entries)])
}

fn entry(key: Value, value: Value) -> Value {
    Value::Object(vec![("key".to_string(), key), ("value".to_string(), value)])
}

/// `key`, then `Key`, then `name`, then `Name`. Null and false are skipped,
/// as with `//`. The value is `value` when that field is present, otherwise
/// `Value`, otherwise null. A null value does not fall through.
pub(super) fn eval_from_entries(input: &Value) -> Result<Vec<Value>, Failure> {
    let items = iterate(input)?;
    let mut pairs = Vec::new();
    for item in items {
        let key = entry_key(&item)?;
        let Value::String(name) = key else {
            return Err(fail(super::parse::key_message(&key)));
        };
        put(&mut pairs, name, entry_value(&item));
    }
    Ok(vec![Value::Object(pairs)])
}

fn entry_key(item: &Value) -> Result<Value, Failure> {
    match item {
        Value::Object(pairs) => {
            for name in ["key", "Key", "name", "Name"] {
                if let Some(value) = field(pairs, name) {
                    if truthy(value) {
                        return Ok(value.clone());
                    }
                }
            }
            Ok(Value::Null)
        }
        Value::Null => Ok(Value::Null),
        other => {
            index_string(other, "key")?;
            Ok(Value::Null)
        }
    }
}

fn entry_value(item: &Value) -> Value {
    let Value::Object(pairs) = item else {
        return Value::Null;
    };
    if let Some(value) = field(pairs, "value") {
        return value.clone();
    }
    if let Some(value) = field(pairs, "Value") {
        return value.clone();
    }
    Value::Null
}

fn field<'a>(pairs: &'a [(String, Value)], name: &str) -> Option<&'a Value> {
    pairs
        .iter()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value)
}

pub(super) fn eval_with_entries(
    update: &Filter,
    input: &Value,
    env: &[(String, Value)],
) -> Result<Vec<Value>, Failure> {
    let entries = eval_to_entries(input)?;
    let mapped = eval_map(update, &entries[0], env)?;
    eval_from_entries(&mapped[0])
}

/// `map` is `[.[] | f]`. A failure inside drops the array.
pub(super) fn eval_map(
    inner: &Filter,
    input: &Value,
    env: &[(String, Value)],
) -> Result<Vec<Value>, Failure> {
    let items = match iterate(input) {
        Ok(items) => items,
        Err(failure) => return Err(failure),
    };
    let mut collected = Vec::new();
    for item in items {
        match eval_in(inner, &item, env) {
            Ok(more) => collected.extend(more),
            Err(failure) => return Err(bare_fail(failure)),
        }
    }
    Ok(vec![Value::Array(collected)])
}

/// A number is unchanged. A string must be one JSON number. The parse error
/// keeps the string's own text, quotes and newlines included.
pub(super) fn eval_tonumber(input: &Value) -> Result<Vec<Value>, Failure> {
    match input {
        Value::Number(_) => Ok(vec![input.clone()]),
        Value::String(text) => one(parse_number_text(text)),
        other => Err(fail(format!(
            "{} ({}) cannot be parsed as a number",
            other.type_name(),
            shown(other)
        ))),
    }
}

pub(super) fn parse_number_text(text: &str) -> Result<Value, String> {
    let parsed = match crate::parse::parse_stream(text.as_bytes()) {
        Ok(parsed) => parsed,
        Err(err) => return Err(format!("{err} (while parsing '{text}')")),
    };
    let suffix = format!(" (while parsing '{text}')");
    if let Some(message) = parsed.error {
        return Err(format!("{message}{suffix}"));
    }
    match parsed.values.as_slice() {
        [] => Err(format!("Expected JSON value{suffix}")),
        [only] => match &only.value {
            Value::Number(raw) => Ok(Value::Number(raw.clone())),
            _ => Err(format!(
                "string ({}) cannot be parsed as a number",
                shown(&Value::String(text.to_string()))
            )),
        },
        _ => Err(format!("Unexpected extra JSON values{suffix}")),
    }
}

/// One separator. An empty walk is `""` and does not use the separator.
/// Null becomes `""`. A number or boolean is compact JSON. An array or
/// object is added as itself, so `+` reports that those cannot be added.
pub(super) fn eval_join(
    arg: &Filter,
    input: &Value,
    env: &[(String, Value)],
) -> Result<Vec<Value>, Failure> {
    let (seps, err) = parts(eval_in(arg, input, env));
    let mut out = Vec::new();
    for sep in seps {
        match join_with(input, &sep) {
            Ok(value) => out.push(value),
            Err(message) => return Err(fresh(out, message)),
        }
    }
    finish(out, err)
}

pub(super) fn join_with(input: &Value, sep: &Value) -> Result<Value, String> {
    let items = match iterate(input) {
        Ok(items) => items,
        Err(failure) => return Err(failure.message),
    };
    if items.is_empty() {
        return Ok(Value::String(String::new()));
    }
    let mut acc = add_values(&Value::String(String::new()), &join_piece(&items[0]))?;
    for item in &items[1..] {
        let with_sep = add_values(&acc, sep)?;
        acc = add_values(&with_sep, &join_piece(item))?;
    }
    Ok(acc)
}

pub(super) fn join_piece(item: &Value) -> Value {
    match item {
        Value::Null => Value::String(String::new()),
        Value::Bool(_) | Value::Number(_) => Value::String(jq_text(item)),
        other => other.clone(),
    }
}

/// Both sides must be strings. One matching copy is dropped. Any other
/// value is returned unchanged, and a later argument still sees the input.
pub(super) fn eval_trim(
    arg: &Filter,
    input: &Value,
    env: &[(String, Value)],
    left: bool,
) -> Result<Vec<Value>, Failure> {
    let (affixes, err) = parts(eval_in(arg, input, env));
    let mut out = Vec::new();
    for affix in affixes {
        out.push(trim_once(input, &affix, left));
    }
    finish(out, err)
}

pub(super) fn trim_once(input: &Value, affix: &Value, left: bool) -> Value {
    let (Value::String(text), Value::String(edge)) = (input, affix) else {
        return input.clone();
    };
    let trimmed = if left {
        text.strip_prefix(edge.as_str())
    } else {
        text.strip_suffix(edge.as_str())
    };
    Value::String(trimmed.unwrap_or(text).to_string())
}
