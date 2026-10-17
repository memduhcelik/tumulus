//! Builtins: length, keys, type, strings, math, dates, try, and error.

use std::cell::RefCell;

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
        stop: None,
    }
}

pub(super) fn eval_debug(input: &Value) -> Result<Vec<Value>, Failure> {
    let wrapped = Value::array(vec![Value::String("DEBUG:".to_string()), input.clone()]);
    let (ascii, sort_keys) = crate::feed::dump_flags();
    let style = crate::print::Style {
        layout: crate::print::Layout::Compact,
        raw: false,
        join: false,
        ascii,
        sort_keys,
    };
    let mut bytes = crate::print::render(&wrapped, style).into_bytes();
    bytes.push(b'\n');
    write_err(&bytes);
    Ok(vec![input.clone()])
}

pub(super) fn eval_stderr(input: &Value) -> Result<Vec<Value>, Failure> {
    let bytes = match input {
        Value::String(text) => text.as_bytes().to_vec(),
        other => {
            let style = crate::print::Style {
                layout: crate::print::Layout::Compact,
                raw: false,
                join: false,
                ascii: false,
                sort_keys: false,
            };
            crate::print::render(other, style).into_bytes()
        }
    };
    write_err(&bytes);
    Ok(vec![input.clone()])
}

fn write_err(bytes: &[u8]) {
    use std::io::Write;
    let mut err = std::io::stderr();
    let _ = err.write_all(bytes);
    let _ = err.flush();
}

pub(super) fn builtins_value() -> Value {
    Value::array(
        super::names::BUILTINS
            .iter()
            .map(|name| Value::String((*name).to_string()))
            .collect(),
    )
}

pub(super) fn eval_num_test(kind: NumTest, input: &Value) -> Result<Vec<Value>, Failure> {
    let flag = match input {
        Value::Number(text) => {
            let n = number::f64_of(text);
            match kind {
                NumTest::Nan => n.is_nan(),
                NumTest::Infinite => n.is_infinite(),
                NumTest::Normal => n.is_normal(),
                // jq's `isfinite` is `type == "number" and (isinfinite | not)`.
                NumTest::Finite => !n.is_infinite(),
            }
        }
        _ => false,
    };
    Ok(vec![Value::Bool(flag)])
}

/// `halt`. The input is not printed.
pub(super) fn stop_quiet() -> Failure {
    Failure {
        partial: Vec::new(),
        message: String::new(),
        caught: None,
        stop: Some(Stop {
            code: 0,
            stderr: Vec::new(),
        }),
    }
}

/// A C cast to `int`, then the low 8 bits. A negative result is status 0.
/// Out-of-range values follow the host, the same way jq's cast does.
fn halt_code(n: f64) -> i32 {
    let code = number::c_cast_i32(n);
    if code < 0 {
        0
    } else {
        code & 255
    }
}

/// Strings are written raw. `null` writes nothing. Anything else is compact JSON and a newline.
fn halt_bytes(value: &Value) -> Vec<u8> {
    match value {
        Value::Null => Vec::new(),
        Value::String(text) => text.as_bytes().to_vec(),
        other => {
            let style = crate::print::Style {
                layout: crate::print::Layout::Compact,
                raw: false,
                join: false,
                ascii: false,
                sort_keys: false,
            };
            let mut bytes = crate::print::render(other, style).into_bytes();
            bytes.push(b'\n');
            bytes
        }
    }
}

fn stop_with(code: i32, value: &Value) -> Failure {
    Failure {
        partial: Vec::new(),
        message: String::new(),
        caught: None,
        stop: Some(Stop {
            code,
            stderr: halt_bytes(value),
        }),
    }
}

/// `halt_error(code)`. The first number stops the program. The stderr text is the input.
pub(super) fn eval_halt_error(
    code: &Filter,
    input: &Value,
    env: &[(String, Value)],
) -> Result<Vec<Value>, Failure> {
    let (values, err) = parts(eval_in(code, input, env));
    if let Some(value) = values.into_iter().next() {
        let Value::Number(text) = &value else {
            return Err(fail(format!(
                "{} ({}) halt_error/1: number required",
                input.type_name(),
                shown(input)
            )));
        };
        return Err(stop_with(halt_code(number::f64_of(text)), input));
    }
    match err {
        Some(failure) => Err(failure),
        None => Ok(Vec::new()),
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
    if failure.stop.is_some() {
        return Err(failure);
    }
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
        math::Out::Pair(first, second) => Value::array(vec![from_f64(first), from_f64(second)]),
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
    key_list(input, true)
}

/// Object keys stay in stored order. A repeated name keeps its first place.
pub(super) fn eval_keys_unsorted(input: &Value) -> Result<Vec<Value>, Failure> {
    key_list(input, false)
}

fn key_list(input: &Value, sorted: bool) -> Result<Vec<Value>, Failure> {
    match input {
        Value::Array(items) => {
            let keys = (0..items.len())
                .map(|index| Value::Number(index.to_string()))
                .collect();
            Ok(vec![Value::array(keys)])
        }
        Value::Object(items) => {
            let mut names: Vec<&str> = Vec::new();
            for (name, _) in items.iter() {
                if !names.contains(&name.as_str()) {
                    names.push(name.as_str());
                }
            }
            if sorted {
                names.sort_unstable();
            }
            let keys = names
                .into_iter()
                .map(|name| Value::String(name.to_string()))
                .collect();
            Ok(vec![Value::array(keys)])
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
            .collect::<Vec<_>>(),
        Value::Object(pairs) => pairs
            .iter()
            .map(|(key, value)| entry(Value::String(key.clone()), value.clone()))
            .collect::<Vec<_>>(),
        other => {
            return Err(fail(format!(
                "{} ({}) has no keys",
                other.type_name(),
                shown(other)
            )));
        }
    };
    Ok(vec![Value::array(entries)])
}

fn entry(key: Value, value: Value) -> Value {
    Value::object(vec![("key".to_string(), key), ("value".to_string(), value)])
}

/// `key`, then `Key`, then `name`, then `Name`. Null and false are skipped,
/// as with `//`. The value is `value` when that field is present, otherwise
/// `Value`, otherwise null. A null value does not fall through.
pub(super) fn eval_from_entries(input: &Value) -> Result<Vec<Value>, Failure> {
    let items = iterate(input)?;
    let mut pairs = Vec::new();
    for item in items {
        let key = entry_key(&item)?;
        let name = match key.into_string() {
            Ok(name) => name,
            Err(key) => return Err(fail(super::parse::key_message(&key))),
        };
        put(&mut pairs, name, entry_value(&item));
    }
    Ok(vec![Value::object(pairs)])
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
    Ok(vec![Value::array(collected)])
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

/// Which occurrence `index`, `rindex`, or `indices` keeps.
pub(super) enum Find {
    Index,
    RIndex,
    Indices,
}

/// Only ASCII letters move. Anything else fails the way `explode` fails.
pub(super) fn eval_ascii_case(input: &Value, up: bool) -> Result<Vec<Value>, Failure> {
    let Value::String(text) = input else {
        return Err(fail("explode input must be a string".to_string()));
    };
    let mapped: String = text
        .chars()
        .map(|ch| {
            if up {
                ch.to_ascii_uppercase()
            } else {
                ch.to_ascii_lowercase()
            }
        })
        .collect();
    Ok(vec![Value::String(mapped)])
}

pub(super) fn eval_explode(input: &Value) -> Result<Vec<Value>, Failure> {
    let Value::String(text) = input else {
        return Err(fail("explode input must be a string".to_string()));
    };
    let points = text
        .chars()
        .map(|ch| Value::Number((ch as u32).to_string()))
        .collect();
    Ok(vec![Value::array(points)])
}

pub(super) fn eval_implode(input: &Value) -> Result<Vec<Value>, Failure> {
    let Value::Array(items) = input else {
        return Err(fail("implode input must be an array".to_string()));
    };
    let mut text = String::new();
    for item in items {
        text.push(implode_point(item)?);
    }
    Ok(vec![Value::String(text)])
}

fn implode_point(item: &Value) -> Result<char, Failure> {
    let Value::Number(raw) = item else {
        return Err(fail(format!(
            "{} ({}) can't be imploded, unicode codepoint needs to be numeric",
            item.type_name(),
            shown(item)
        )));
    };
    let number = number::f64_of(raw);
    if number.is_nan() {
        return Err(fail(format!(
            "number ({}) can't be imploded, unicode codepoint needs to be numeric",
            shown(item)
        )));
    }
    Ok(codepoint(number))
}

/// Truncate toward zero. Out of range and UTF-16 surrogates become U+FFFD.
fn codepoint(number: f64) -> char {
    if !number.is_finite() {
        return '\u{FFFD}';
    }
    let truncated = number.trunc();
    if truncated < 0.0 || truncated > f64::from(0x10_FFFF) {
        return '\u{FFFD}';
    }
    char::from_u32(truncated as u32).unwrap_or('\u{FFFD}')
}

pub(super) fn eval_utf8_byte_length(input: &Value) -> Result<Vec<Value>, Failure> {
    let Value::String(text) = input else {
        return Err(fail(format!(
            "{} ({}) only strings have UTF-8 byte length",
            input.type_name(),
            shown(input)
        )));
    };
    Ok(vec![Value::Number(text.len().to_string())])
}

/// `end` chooses `endswith`. Both values have to be strings.
pub(super) fn eval_affix(
    arg: &Filter,
    input: &Value,
    env: &[(String, Value)],
    end: bool,
) -> Result<Vec<Value>, Failure> {
    let (needles, err) = parts(eval_in(arg, input, env));
    let mut out = Vec::new();
    for needle in &needles {
        match affix(input, needle, end) {
            Ok(flag) => out.push(Value::Bool(flag)),
            Err(message) => return Err(fresh(out, message)),
        }
    }
    finish(out, err)
}

fn affix(input: &Value, needle: &Value, end: bool) -> Result<bool, String> {
    let name = if end { "endswith" } else { "startswith" };
    let (Value::String(text), Value::String(edge)) = (input, needle) else {
        return Err(format!("{name}() requires string inputs"));
    };
    Ok(if end {
        text.ends_with(edge.as_str())
    } else {
        text.starts_with(edge.as_str())
    })
}

pub(super) fn eval_find(
    arg: &Filter,
    input: &Value,
    env: &[(String, Value)],
    kind: Find,
) -> Result<Vec<Value>, Failure> {
    let (needles, err) = parts(eval_in(arg, input, env));
    let mut out = Vec::new();
    for needle in &needles {
        match locate(input, needle) {
            Ok(found) => match kind {
                Find::Indices => out.extend(found),
                Find::Index => out.extend(pick_first(&found)?),
                Find::RIndex => out.extend(pick_last(&found)?),
            },
            Err(failure) => return Err(with_partial(out, failure)),
        }
    }
    finish(out, err)
}

/// Positions in a string or array, or the values of `.[needle]` otherwise.
fn locate(input: &Value, needle: &Value) -> Result<Vec<Value>, Failure> {
    match input {
        Value::Array(items) => Ok(vec![positions(array_hits(items, needle))]),
        Value::String(text) => match needle {
            Value::String(edge) => Ok(vec![positions(byte_hits(text, edge))]),
            _ => at_key(input, needle),
        },
        _ => at_key(input, needle),
    }
}

fn positions(hits: Vec<usize>) -> Value {
    Value::array(
        hits.into_iter()
            .map(|hit| Value::Number(hit.to_string()))
            .collect(),
    )
}

/// `indices` yields these values. `index` keeps `.[0]` of each. `rindex`
/// keeps `.[-1:][0]` of each.
fn pick_first(values: &[Value]) -> Result<Vec<Value>, Failure> {
    let mut out = Vec::new();
    for value in values {
        out.extend(index_number(value, &ArrayIndex::At(0))?);
    }
    Ok(out)
}

fn pick_last(values: &[Value]) -> Result<Vec<Value>, Failure> {
    let mut out = Vec::new();
    for value in values {
        let sliced = slice(value, Some("-1"), None)?;
        for piece in sliced {
            out.extend(index_number(&piece, &ArrayIndex::At(0))?);
        }
    }
    Ok(out)
}

/// Object keys keep the last copy, which is what `.[name]` does in jq.
fn at_key(input: &Value, key: &Value) -> Result<Vec<Value>, Failure> {
    if let (Value::Object(items), Value::String(name)) = (input, key) {
        let found = items
            .iter()
            .rev()
            .find(|(item, _)| item == name)
            .map(|(_, value)| value.clone())
            .unwrap_or(Value::Null);
        return Ok(vec![found]);
    }
    index_by(input, key)
}

fn array_hits(items: &[Value], needle: &Value) -> Vec<usize> {
    let Value::Array(seq) = needle else {
        return items
            .iter()
            .enumerate()
            .filter(|(_, item)| same(item, needle))
            .map(|(index, _)| index)
            .collect();
    };
    if seq.is_empty() || seq.len() > items.len() {
        return Vec::new();
    }
    let last = items.len() - seq.len();
    let mut hits = Vec::new();
    for index in 0..=last {
        if items[index..index + seq.len()]
            .iter()
            .zip(seq.iter())
            .all(|(item, want)| same(item, want))
        {
            hits.push(index);
        }
    }
    hits
}

fn byte_hits(text: &str, needle: &str) -> Vec<usize> {
    let hay = text.as_bytes();
    let want = needle.as_bytes();
    if want.is_empty() || want.len() > hay.len() {
        return Vec::new();
    }
    let last = hay.len() - want.len();
    let mut hits = Vec::new();
    for index in 0..=last {
        if &hay[index..index + want.len()] == want {
            hits.push(index);
        }
    }
    hits
}

fn same(left: &Value, right: &Value) -> bool {
    cmp_value(left, right) == 0
}

pub(super) fn eval_named_format(name: &str, input: &Value) -> Result<Vec<Value>, Failure> {
    Ok(vec![Value::String(apply_format(name, input)?)])
}

/// `@name "..."`. The rightmost piece is the outer loop, and everything to
/// its left runs again for each of its values. That is how `+` orders them.
pub(super) fn eval_format_string(
    name: &str,
    pieces: &[Piece],
    input: &Value,
    env: &[(String, Value)],
) -> Result<Vec<Value>, Failure> {
    let out = RefCell::new(Vec::new());
    match walk_format(name, pieces, input, env, &out) {
        Ok(()) => Ok(out.into_inner()),
        Err(failure) => Err(with_partial(out.into_inner(), failure)),
    }
}

fn walk_format(
    name: &str,
    pieces: &[Piece],
    input: &Value,
    env: &[(String, Value)],
    out: &RefCell<Vec<Value>>,
) -> Result<(), Failure> {
    if pieces.is_empty() {
        out.borrow_mut().push(Value::String(String::new()));
        return Ok(());
    }
    let last = pieces.len() - 1;
    stream_format_piece(name, &pieces[last], &pieces[..last], input, env, out)
}

fn stream_format_piece(
    name: &str,
    piece: &Piece,
    head: &[Piece],
    input: &Value,
    env: &[(String, Value)],
    out: &RefCell<Vec<Value>>,
) -> Result<(), Failure> {
    match piece {
        Piece::Text(text) => append_format(name, head, text, input, env, out),
        Piece::Interp(filter) => {
            let stop = RefCell::new(None);
            let streamed = super::stream::feed(filter, input, env, &mut |value| {
                if stop.borrow().is_some() {
                    return true;
                }
                match apply_format(name, value) {
                    Ok(text) => match append_format(name, head, &text, input, env, out) {
                        Ok(()) => false,
                        Err(err) => {
                            *stop.borrow_mut() = Some(err);
                            true
                        }
                    },
                    Err(err) => {
                        *stop.borrow_mut() = Some(err);
                        true
                    }
                }
            });
            if let Some(failure) = stop.into_inner() {
                return Err(failure);
            }
            match streamed {
                Ok(_) => Ok(()),
                Err(mut failure) => {
                    for value in std::mem::take(&mut failure.partial) {
                        let text = apply_format(name, &value)?;
                        append_format(name, head, &text, input, env, out)?;
                    }
                    Err(failure)
                }
            }
        }
    }
}

fn append_format(
    name: &str,
    head: &[Piece],
    right: &str,
    input: &Value,
    env: &[(String, Value)],
    out: &RefCell<Vec<Value>>,
) -> Result<(), Failure> {
    let mark = out.borrow().len();
    let walked = walk_format(name, head, input, env, out);
    {
        let mut buf = out.borrow_mut();
        for item in &mut buf[mark..] {
            if let Value::String(text) = item {
                text.push_str(right);
            }
        }
    }
    walked
}

fn apply_format(name: &str, input: &Value) -> Result<String, Failure> {
    match name {
        "text" => Ok(jq_text(input)),
        "json" => Ok(json_text(input)),
        "html" => Ok(html_escape(&jq_text(input))),
        "uri" => Ok(uri_escape(&jq_text(input))),
        "base64" => Ok(base64_encode(&jq_text(input))),
        "base64d" => base64_decode(&jq_text(input)),
        "sh" => shell_escape(input),
        "csv" => table_row(input, ",", "csv"),
        "tsv" => table_row(input, "\t", "tsv"),
        _ => Err(fail(format!("{name} is not a valid format"))),
    }
}

fn json_text(input: &Value) -> String {
    let style = crate::print::Style {
        layout: crate::print::Layout::Compact,
        ..crate::print::Style::default()
    };
    crate::print::render(input, style)
}

fn html_escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '\0' => out.push_str("\\0"),
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '\'' => out.push_str("&apos;"),
            '"' => out.push_str("&quot;"),
            other => out.push(other),
        }
    }
    out
}

fn uri_escape(text: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut out = String::new();
    for byte in text.as_bytes() {
        if byte.is_ascii_alphanumeric() || matches!(*byte, b'-' | b'.' | b'_' | b'~') {
            out.push(*byte as char);
        } else {
            out.push('%');
            out.push(HEX[(byte >> 4) as usize] as char);
            out.push(HEX[(byte & 0x0f) as usize] as char);
        }
    }
    out
}

fn shell_escape(input: &Value) -> Result<String, Failure> {
    match input {
        Value::Array(items) => {
            let mut out = String::new();
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push(' ');
                }
                out.push_str(&shell_scalar(item)?);
            }
            Ok(out)
        }
        other => shell_scalar(other),
    }
}

fn shell_scalar(input: &Value) -> Result<String, Failure> {
    match input {
        Value::String(text) => Ok(shell_quote(text)),
        Value::Array(_) => Err(fail(format!(
            "array ({}) can not be escaped for shell",
            shown(input)
        ))),
        Value::Object(_) => Err(fail(format!(
            "object ({}) can not be escaped for shell",
            shown(input)
        ))),
        other => Ok(jq_text(other)),
    }
}

fn shell_quote(text: &str) -> String {
    let mut out = String::from("'");
    for ch in text.chars() {
        match ch {
            '\0' => out.push_str("\\0"),
            '\'' => out.push_str("'\\''"),
            other => out.push(other),
        }
    }
    out.push('\'');
    out
}

fn blank_field(value: &Value) -> bool {
    match value {
        Value::Null => true,
        Value::Number(text) => matches!(number::real(text), number::Real::Nan),
        _ => false,
    }
}

/// `kind` is `csv` or `tsv`. A bad element says `csv row` for both.
fn table_row(input: &Value, sep: &str, kind: &str) -> Result<String, Failure> {
    let Value::Array(items) = input else {
        return Err(fail(format!(
            "{} ({}) cannot be {kind}-formatted, only array",
            input.type_name(),
            shown(input)
        )));
    };
    let mut out = String::new();
    for (index, item) in items.iter().enumerate() {
        if index > 0 {
            out.push_str(sep);
        }
        if blank_field(item) {
            continue;
        }
        match item {
            Value::Bool(_) | Value::Number(_) => out.push_str(&jq_text(item)),
            Value::String(text) => out.push_str(&field_text(text, kind == "tsv")),
            other => {
                return Err(fail(format!(
                    "{} ({}) is not valid in a csv row",
                    other.type_name(),
                    shown(other)
                )));
            }
        }
    }
    Ok(out)
}

fn field_text(text: &str, tsv: bool) -> String {
    if tsv {
        let mut out = String::new();
        for ch in text.chars() {
            match ch {
                '\\' => out.push_str("\\\\"),
                '\n' => out.push_str("\\n"),
                '\r' => out.push_str("\\r"),
                '\t' => out.push_str("\\t"),
                '\0' => out.push_str("\\0"),
                other => out.push(other),
            }
        }
        return out;
    }
    let mut out = String::from("\"");
    for ch in text.chars() {
        match ch {
            '"' => out.push_str("\"\""),
            '\0' => out.push_str("\\0"),
            other => out.push(other),
        }
    }
    out.push('"');
    out
}

fn base64_encode(text: &str) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let bytes = text.as_bytes();
    let mut out = String::new();
    let mut index = 0;
    while index + 3 <= bytes.len() {
        let n = (u32::from(bytes[index]) << 16)
            | (u32::from(bytes[index + 1]) << 8)
            | u32::from(bytes[index + 2]);
        out.push(TABLE[((n >> 18) & 63) as usize] as char);
        out.push(TABLE[((n >> 12) & 63) as usize] as char);
        out.push(TABLE[((n >> 6) & 63) as usize] as char);
        out.push(TABLE[(n & 63) as usize] as char);
        index += 3;
    }
    let rest = &bytes[index..];
    if rest.len() == 1 {
        let n = u32::from(rest[0]) << 16;
        out.push(TABLE[((n >> 18) & 63) as usize] as char);
        out.push(TABLE[((n >> 12) & 63) as usize] as char);
        out.push('=');
        out.push('=');
    } else if rest.len() == 2 {
        let n = (u32::from(rest[0]) << 16) | (u32::from(rest[1]) << 8);
        out.push(TABLE[((n >> 18) & 63) as usize] as char);
        out.push(TABLE[((n >> 12) & 63) as usize] as char);
        out.push(TABLE[((n >> 6) & 63) as usize] as char);
        out.push('=');
    }
    out
}

/// Stop at the first `=`. One leftover sextet is the trailing-byte error.
/// Any other count of leftover bits is padding and is dropped.
fn base64_decode(text: &str) -> Result<String, Failure> {
    let mut bytes = Vec::new();
    let mut acc: u32 = 0;
    let mut bits: u32 = 0;
    for ch in text.chars() {
        if ch == '=' {
            break;
        }
        let Some(digit) = base64_digit(ch) else {
            return Err(base64_invalid(text));
        };
        acc = (acc << 6) | digit;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            bytes.push((acc >> bits) as u8);
            acc &= if bits == 0 { 0 } else { (1 << bits) - 1 };
        }
    }
    if bits == 6 {
        return Err(base64_trailing(text));
    }
    Ok(crate::parse::repair_utf8(&bytes))
}

fn base64_digit(ch: char) -> Option<u32> {
    match ch {
        'A'..='Z' => Some(u32::from(ch) - u32::from('A')),
        'a'..='z' => Some(u32::from(ch) - u32::from('a') + 26),
        '0'..='9' => Some(u32::from(ch) - u32::from('0') + 52),
        '+' => Some(62),
        '/' => Some(63),
        _ => None,
    }
}

fn base64_invalid(text: &str) -> Failure {
    fail(format!(
        "string ({}) is not valid base64 data",
        shown(&Value::String(text.to_string()))
    ))
}

fn base64_trailing(text: &str) -> Failure {
    fail(format!(
        "string ({}) trailing base64 byte found",
        shown(&Value::String(text.to_string()))
    ))
}

/// One JSON text. The parse message is jq's, then the original string.
pub(super) fn eval_fromjson(input: &Value) -> Result<Vec<Value>, Failure> {
    match input {
        Value::String(text) => one(parse_any_text(text)),
        other => Err(fail(format!(
            "{} ({}) only strings can be parsed",
            other.type_name(),
            shown(other)
        ))),
    }
}

pub(super) fn parse_any_text(text: &str) -> Result<Value, String> {
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
        [only] => Ok(only.value.clone()),
        _ => Err(format!("Unexpected extra JSON values{suffix}")),
    }
}

/// Each separator is its own split. A failure keeps the splits already made.
pub(super) fn eval_split(
    sep: &Filter,
    input: &Value,
    env: &[(String, Value)],
) -> Result<Vec<Value>, Failure> {
    let (seps, err) = parts(eval_in(sep, input, env));
    let mut out = Vec::new();
    for sep in seps {
        match split_one(input, &sep) {
            Ok(value) => out.push(value),
            Err(message) => return Err(fresh(out, message)),
        }
    }
    finish(out, err)
}

fn split_one(input: &Value, sep: &Value) -> Result<Value, String> {
    match (input, sep) {
        (Value::String(text), Value::String(sep)) => Ok(split_string(text, sep)),
        _ => Err("split input and separator must be strings".to_string()),
    }
}

pub(super) fn eval_contains(
    needle: &Filter,
    input: &Value,
    env: &[(String, Value)],
) -> Result<Vec<Value>, Failure> {
    let (needles, err) = parts(eval_in(needle, input, env));
    let mut out = Vec::new();
    for needle in needles {
        match contains_top(input, &needle) {
            Ok(flag) => out.push(Value::Bool(flag)),
            Err(message) => return Err(fresh(out, message)),
        }
    }
    finish(out, err)
}

/// `inside(haystack)` asks whether `haystack` contains the input.
pub(super) fn eval_inside(
    haystack: &Filter,
    input: &Value,
    env: &[(String, Value)],
) -> Result<Vec<Value>, Failure> {
    let (haystacks, err) = parts(eval_in(haystack, input, env));
    let mut out = Vec::new();
    for haystack in haystacks {
        match contains_top(&haystack, input) {
            Ok(flag) => out.push(Value::Bool(flag)),
            Err(message) => return Err(fresh(out, message)),
        }
    }
    finish(out, err)
}

/// Different types fail here. An unequal pair of booleans fails here too.
/// A mismatch inside an array or object is false, which `contains_rec` does.
fn contains_top(hay: &Value, needle: &Value) -> Result<bool, String> {
    let unequal_bools =
        matches!((hay, needle), (Value::Bool(left), Value::Bool(right)) if left != right);
    if hay.type_name() != needle.type_name() || unequal_bools {
        return Err(format!(
            "{} ({}) and {} ({}) cannot have their containment checked",
            hay.type_name(),
            shown(hay),
            needle.type_name(),
            shown(needle)
        ));
    }
    Ok(contains_rec(hay, needle))
}

fn contains_rec(hay: &Value, needle: &Value) -> bool {
    match (hay, needle) {
        (Value::String(hay), Value::String(needle)) => needle.is_empty() || hay.contains(needle),
        (Value::Array(hay), Value::Array(needle)) => needle
            .iter()
            .all(|item| hay.iter().any(|element| contains_rec(element, item))),
        (Value::Object(hay), Value::Object(needle)) => needle.iter().all(|(key, want)| {
            hay.iter()
                .rev()
                .find(|(name, _)| name == key)
                .is_some_and(|(_, have)| contains_rec(have, want))
        }),
        _ => cmp_value(hay, needle) == 0,
    }
}

pub(super) fn eval_flatten(
    depth: Option<&Filter>,
    input: &Value,
    env: &[(String, Value)],
) -> Result<Vec<Value>, Failure> {
    let Some(depth) = depth else {
        return flatten_one(input, None)
            .map(|value| vec![value])
            .map_err(fail);
    };
    let (depths, err) = parts(eval_in(depth, input, env));
    let mut out = Vec::new();
    for depth in depths {
        match flatten_one(input, Some(&depth)) {
            Ok(value) => out.push(value),
            Err(message) => return Err(fresh(out, message)),
        }
    }
    finish(out, err)
}

enum Flat {
    /// Do not enter nested arrays.
    Keep,
    /// Enter every nested array.
    Full,
    /// Enter one level and pass `depth - 1` down.
    Down,
}

fn flatten_one(input: &Value, depth: Option<&Value>) -> Result<Value, String> {
    let (mode, depth) = match depth {
        None => (Flat::Full, Value::Null),
        Some(depth) => {
            let zero = Value::Number("0".to_string());
            if cmp_value(depth, &zero) < 0 {
                return Err("flatten depth must not be negative".to_string());
            }
            (flat_mode(depth), depth.clone())
        }
    };
    let mut out = Vec::new();
    let mut stack = vec![FlatFrame {
        items: children(input)?,
        index: 0,
        mode,
        depth,
    }];
    loop {
        let Some(top) = stack.last() else {
            break;
        };
        if top.index >= top.items.len() {
            stack.pop();
            continue;
        }
        let at = top.index;
        let item = top.items[at].clone();
        let mode = match top.mode {
            Flat::Keep => Flat::Keep,
            Flat::Full => Flat::Full,
            Flat::Down => Flat::Down,
        };
        let depth = top.depth.clone();
        stack.last_mut().expect("frame").index = at + 1;
        if !matches!(item, Value::Array(_)) {
            out.push(item);
            continue;
        }
        match mode {
            Flat::Keep => out.push(item),
            Flat::Full => stack.push(FlatFrame {
                items: children(&item)?,
                index: 0,
                mode: Flat::Full,
                depth,
            }),
            Flat::Down => {
                let one = Value::Number("1".to_string());
                let next = sub_values(&depth, &one)?;
                stack.push(FlatFrame {
                    items: children(&item)?,
                    index: 0,
                    mode: flat_mode(&next),
                    depth: next,
                });
            }
        }
    }
    Ok(Value::array(out))
}

struct FlatFrame {
    items: Vec<Value>,
    index: usize,
    mode: Flat,
    depth: Value,
}

fn children(value: &Value) -> Result<Vec<Value>, String> {
    iterate(value).map_err(|failure| failure.message)
}

/// A positive number that is not an integer flattens every level.
/// A string, array, or object is greater than 0, so entering an array
/// subtracts and reports that those values cannot be subtracted.
fn flat_mode(depth: &Value) -> Flat {
    let zero = Value::Number("0".to_string());
    if let Value::Number(text) = depth {
        if cmp_value(depth, &zero) == 0 {
            return Flat::Keep;
        }
        let n = number::f64_of(text);
        let whole = n.is_infinite() || (n.is_finite() && n.floor() == n);
        if !whole {
            return Flat::Full;
        }
        return Flat::Down;
    }
    if cmp_value(depth, &zero) > 0 {
        Flat::Down
    } else {
        Flat::Keep
    }
}

/// The first decisive condition stops the generator. `all` of nothing is true.
pub(super) fn eval_quant(
    all: bool,
    stream: &Filter,
    cond: &Filter,
    input: &Value,
    env: &[(String, Value)],
) -> Result<Vec<Value>, Failure> {
    let decided = std::cell::Cell::new(None);
    let failure = std::cell::RefCell::new(None);
    let ran = super::stream::feed(stream, input, env, &mut |item| {
        let mut halt = false;
        let inner = super::stream::feed(cond, item, env, &mut |flag| {
            let yes = truthy(flag);
            if all {
                if !yes {
                    decided.set(Some(false));
                    halt = true;
                    return true;
                }
            } else if yes {
                decided.set(Some(true));
                halt = true;
                return true;
            }
            false
        });
        if let Err(err) = inner {
            *failure.borrow_mut() = Some(err);
            return true;
        }
        halt
    });
    if let Some(err) = failure.into_inner() {
        return Err(err);
    }
    ran?;
    Ok(vec![Value::Bool(decided.get().unwrap_or(all))])
}

pub(super) fn eval_isempty(
    expr: &Filter,
    input: &Value,
    env: &[(String, Value)],
) -> Result<Vec<Value>, Failure> {
    let saw = std::cell::Cell::new(false);
    let ran = super::stream::feed(expr, input, env, &mut |_| {
        saw.set(true);
        true
    });
    if saw.get() {
        return Ok(vec![Value::Bool(false)]);
    }
    ran?;
    Ok(vec![Value::Bool(true)])
}

/// The first value wins, and the rest of `expr` is not run. No value deletes.
pub(super) fn eval_map_values(
    expr: &Filter,
    input: &Value,
    env: &[(String, Value)],
) -> Result<Vec<Value>, Failure> {
    match input {
        Value::Array(items) => {
            let mut out = Vec::new();
            for item in items.iter() {
                match first_value(expr, item, env)? {
                    Some(value) => out.push(value),
                    None => {}
                }
            }
            Ok(vec![Value::array(out)])
        }
        Value::Object(pairs) => {
            let mut out = Vec::new();
            for (key, value) in pairs.iter() {
                if let Some(value) = first_value(expr, value, env)? {
                    out.push((key.clone(), value));
                }
            }
            Ok(vec![Value::object(out)])
        }
        other => Err(fail(format!(
            "Cannot iterate over {} ({})",
            other.type_name(),
            shown(other)
        ))),
    }
}

fn first_value(
    expr: &Filter,
    input: &Value,
    env: &[(String, Value)],
) -> Result<Option<Value>, Failure> {
    let found = std::cell::RefCell::new(None);
    let ran = super::stream::feed(expr, input, env, &mut |value| {
        *found.borrow_mut() = Some(value.clone());
        true
    });
    if let Some(value) = found.into_inner() {
        return Ok(Some(value));
    }
    match ran {
        Ok(_) => Ok(None),
        Err(err) => Err(bare_fail(err)),
    }
}

/// Path and assignment see the same filter jq builds for `any` and `all`.
/// The condition runs before `if`, so a path inside it is kept. Value
/// results use `eval_quant`, which stops before a later side effect.
pub(super) fn expand_quant(all: bool, stream: &Filter, cond: &Filter) -> Filter {
    let yes = if all {
        Filter::Empty
    } else {
        Filter::Literal(Value::Bool(true))
    };
    let no = if all {
        Filter::Literal(Value::Bool(false))
    } else {
        Filter::Empty
    };
    // `generator | condition | if . then ...`
    let body = Filter::Pipe(
        Box::new(stream.clone()),
        Box::new(Filter::Pipe(
            Box::new(cond.clone()),
            Box::new(Filter::If {
                cond: Box::new(Filter::Identity),
                yes: Box::new(yes),
                no: Box::new(no),
            }),
        )),
    );
    if all {
        Filter::First(Some(Box::new(Filter::Comma(
            Box::new(body),
            Box::new(Filter::Literal(Value::Bool(true))),
        ))))
    } else {
        Filter::Alt(
            Box::new(Filter::First(Some(Box::new(body)))),
            Box::new(Filter::Literal(Value::Bool(false))),
        )
    }
}

/// `first((expr | false), true)`. A value stops before `true`.
pub(super) fn expand_isempty(expr: &Filter) -> Filter {
    let pipe = Filter::Pipe(
        Box::new(expr.clone()),
        Box::new(Filter::Literal(Value::Bool(false))),
    );
    Filter::First(Some(Box::new(Filter::Comma(
        Box::new(pipe),
        Box::new(Filter::Literal(Value::Bool(true))),
    ))))
}

/// `IN(stream)` compares the input with each value of `stream`.
/// `IN(src; stream)` compares each value of `src` with each value of `stream`.
/// `stream` runs on the outside. A match stops both, and a failure in the
/// value being compared stops before the next value of `stream`.
pub(super) fn eval_member(
    src: Option<&Filter>,
    stream: &Filter,
    input: &Value,
    env: &[(String, Value)],
) -> Result<Vec<Value>, Failure> {
    let found = if let Some(src) = src {
        any_equal(stream, src, input, env)?
    } else {
        any_equal(&Filter::Identity, stream, input, env)?
    };
    Ok(vec![Value::Bool(found)])
}

/// The right-hand filter is the outer loop. The left-hand filter runs again
/// for each of its values, in the caller's scope.
fn any_equal(
    outer: &Filter,
    inner: &Filter,
    input: &Value,
    env: &[(String, Value)],
) -> Result<bool, Failure> {
    let found = std::cell::Cell::new(false);
    let failure = RefCell::new(None);
    let ran = super::stream::feed(outer, input, env, &mut |right| {
        let right = right.clone();
        let mut halt = false;
        let left = super::stream::feed(inner, input, env, &mut |left| {
            if cmp_value(left, &right) == 0 {
                found.set(true);
                halt = true;
                return true;
            }
            false
        });
        if let Err(err) = left {
            *failure.borrow_mut() = Some(err);
            return true;
        }
        halt
    });
    if let Some(err) = failure.into_inner() {
        return Err(err);
    }
    ran?;
    Ok(found.get())
}

/// One object. The key filter runs on each value, with the caller's
/// variables. A string is the key. Anything else uses its compact JSON.
/// No key skips that value. A later value replaces an earlier one and
/// leaves the key where it first appeared.
pub(super) fn eval_index_by(
    stream: Option<&Filter>,
    key: &Filter,
    input: &Value,
    env: &[(String, Value)],
) -> Result<Vec<Value>, Failure> {
    let iterate = Filter::Iterate;
    let stream = stream.unwrap_or(&iterate);
    let pairs = RefCell::new(Vec::new());
    let failure = RefCell::new(None);
    let ran = super::stream::feed(stream, input, env, &mut |row| {
        let row = row.clone();
        let keyed = super::stream::feed(key, &row, env, &mut |item| {
            let name = index_key(item);
            put(&mut pairs.borrow_mut(), name, row.clone());
            false
        });
        if let Err(err) = keyed {
            *failure.borrow_mut() = Some(err);
            return true;
        }
        false
    });
    if let Some(err) = failure.into_inner() {
        return Err(err);
    }
    ran?;
    Ok(vec![Value::object(pairs.into_inner())])
}

fn index_key(value: &Value) -> String {
    if let Value::String(text) = value {
        return text.clone();
    }
    match eval_tojson(value) {
        Ok(items) => match items.first() {
            Some(Value::String(text)) => text.clone(),
            _ => String::new(),
        },
        Err(_) => String::new(),
    }
}
