//! Paths, slices, iteration, recurse, pipe, comma, operators, and values.

use crate::number::{self, array_index, slice_bound};
use crate::print::{self, Layout, Style};
use crate::value::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

#[derive(Clone, Debug)]
pub enum Filter {
    Identity,
    Field(String),
    Index(ArrayIndex),
    /// `.[start:end]`. `None` means that side was left off.
    Slice {
        start: Option<String>,
        end: Option<String>,
    },
    Iterate,
    /// `..`, the value and then every value inside it.
    Recurse,
    Pipe(Box<Filter>, Box<Filter>),
    Comma(Box<Filter>, Box<Filter>),
    Optional(Box<Filter>),
    Literal(Value),
    /// Pieces of a string, with `\( )` between them.
    Interpolate(Vec<Piece>),
    /// `[ expr ]` gathers every output of `expr` into one array.
    Collect(Box<Filter>),
    Object(Vec<ObjField>),
    /// `base[key]`, with both sides run on the same input.
    At {
        base: Box<Filter>,
        key: Box<Filter>,
    },
    Bin {
        op: Op,
        left: Box<Filter>,
        right: Box<Filter>,
    },
    /// A leading `-`.
    Neg(Box<Filter>),
    /// `not`, read from the current input.
    Not,
    And(Box<Filter>, Box<Filter>),
    Or(Box<Filter>, Box<Filter>),
    /// `//`
    Alt(Box<Filter>, Box<Filter>),
    If {
        cond: Box<Filter>,
        yes: Box<Filter>,
        no: Box<Filter>,
    },
}

/// A piece of an interpolated string.
#[derive(Clone, Debug)]
pub enum Piece {
    Text(String),
    Interp(Box<Filter>),
}

/// One field of an object constructor.
#[derive(Clone, Debug)]
pub enum ObjField {
    /// `{key: value}`. `key` may produce more than one string.
    Pair {
        key: Box<Filter>,
        value: Box<Filter>,
    },
    /// `{"key"}` copies `.["key"]` for each string the key expression yields.
    Lookup(Box<Filter>),
}

/// Why a program was rejected.
pub enum ProgramError {
    /// The program is not in the fragment this version accepts.
    Syntax,
    /// jq's own message for a bad object key or a bad string escape.
    /// The text is the whole stderr, including the trailing newline.
    Report(String),
}

#[derive(Clone, Debug)]
pub enum ArrayIndex {
    At(i64),
    /// Finite, but too far away to be an element of any array we can hold.
    OutOfRange,
}

pub struct Failure {
    pub partial: Vec<Value>,
    pub message: String,
}

pub fn compile(source: &str) -> Result<Filter, ProgramError> {
    let mut parser = Parser {
        src: source,
        index: 0,
    };
    parser.skip_ws();
    let filter = parser.parse_pipe()?;
    parser.skip_ws();
    if !parser.eof() {
        return Err(ProgramError::Syntax);
    }
    Ok(filter)
}

pub fn eval(filter: &Filter, input: &Value) -> Result<Vec<Value>, Failure> {
    match filter {
        Filter::Identity => Ok(vec![input.clone()]),
        Filter::Field(key) => index_string(input, key),
        Filter::Index(index) => index_number(input, index),
        Filter::Slice { start, end } => slice(input, start.as_deref(), end.as_deref()),
        Filter::Iterate => iterate(input),
        Filter::Recurse => Ok(recurse(input)),
        Filter::Optional(inner) => match eval(inner, input) {
            Ok(values) => Ok(values),
            Err(failure) => Ok(failure.partial),
        },
        Filter::Pipe(left, right) => {
            // Values already produced on the left are still fed to the right.
            // A right-hand error stops before the left-hand error is reported.
            let (inputs, left_err) = parts(eval(left, input));
            let mut out = Vec::new();
            for value in inputs {
                match eval(right, &value) {
                    Ok(more) => out.extend(more),
                    Err(failure) => {
                        out.extend(failure.partial);
                        return Err(Failure {
                            partial: out,
                            message: failure.message,
                        });
                    }
                }
            }
            finish(out, left_err)
        }
        Filter::Comma(left, right) => {
            let mut out = eval(left, input)?;
            match eval(right, input) {
                Ok(more) => out.extend(more),
                Err(failure) => {
                    out.extend(failure.partial);
                    return Err(Failure {
                        partial: out,
                        message: failure.message,
                    });
                }
            }
            Ok(out)
        }
        Filter::Literal(value) => Ok(vec![value.clone()]),
        Filter::Interpolate(pieces) => eval_interpolate(pieces, input),
        Filter::Collect(inner) => match eval(inner, input) {
            Ok(items) => Ok(vec![Value::Array(items)]),
            Err(failure) => Err(Failure {
                partial: Vec::new(),
                message: failure.message,
            }),
        },
        Filter::Object(fields) => eval_object(fields, input),
        Filter::At { base, key } => eval_at(base, key, input),
        Filter::Bin { op, left, right } => eval_bin(*op, left, right, input),
        Filter::Neg(inner) => eval_neg(inner, input),
        Filter::Not => Ok(vec![Value::Bool(!truthy(input))]),
        Filter::And(left, right) => eval_and(left, right, input),
        Filter::Or(left, right) => eval_or(left, right, input),
        Filter::Alt(left, right) => eval_alt(left, right, input),
        Filter::If { cond, yes, no } => eval_if(cond, yes, no, input),
    }
}

fn index_string(input: &Value, key: &str) -> Result<Vec<Value>, Failure> {
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

fn index_number(input: &Value, index: &ArrayIndex) -> Result<Vec<Value>, Failure> {
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

fn nth(items: &[Value], index: i64) -> Value {
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

fn slice(input: &Value, start: Option<&str>, end: Option<&str>) -> Result<Vec<Value>, Failure> {
    match input {
        Value::Null => Ok(vec![Value::Null]),
        Value::Array(items) => {
            let (from, to) = slice_range(start, end, items.len());
            Ok(vec![Value::Array(items[from..to].to_vec())])
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

fn slice_range(start: Option<&str>, end: Option<&str>, len: usize) -> (usize, usize) {
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

fn recurse(input: &Value) -> Vec<Value> {
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

fn iterate(input: &Value) -> Result<Vec<Value>, Failure> {
    match input {
        Value::Array(items) => Ok(items.clone()),
        Value::Object(items) => Ok(items.iter().map(|(_, value)| value.clone()).collect()),
        other => Err(fail(format!(
            "Cannot iterate over {} ({})",
            other.type_name(),
            shown(other)
        ))),
    }
}

fn shown(value: &Value) -> String {
    let style = Style {
        layout: Layout::Compact,
        ..Style::default()
    };
    let rendered = print::render(value, style);
    let bytes = rendered.as_bytes();
    if bytes.len() <= 14 {
        return rendered;
    }
    // Keep 11 bytes. An incomplete UTF-8 sequence becomes U+FFFD.
    let head = &bytes[..11];
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

fn utf8_width(lead: u8) -> usize {
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

fn fail(message: String) -> Failure {
    Failure {
        partial: Vec::new(),
        message,
    }
}

fn parts(result: Result<Vec<Value>, Failure>) -> (Vec<Value>, Option<String>) {
    match result {
        Ok(values) => (values, None),
        Err(failure) => (failure.partial, Some(failure.message)),
    }
}

fn truthy(value: &Value) -> bool {
    !matches!(value, Value::Null | Value::Bool(false))
}

fn finish(out: Vec<Value>, err: Option<String>) -> Result<Vec<Value>, Failure> {
    match err {
        Some(message) => Err(Failure {
            partial: out,
            message,
        }),
        None => Ok(out),
    }
}

fn eval_neg(inner: &Filter, input: &Value) -> Result<Vec<Value>, Failure> {
    let (values, err) = parts(eval(inner, input));
    let mut out = Vec::new();
    for value in values {
        match negate_value(&value) {
            Ok(next) => out.push(next),
            Err(message) => {
                return Err(Failure {
                    partial: out,
                    message,
                })
            }
        }
    }
    finish(out, err)
}

fn eval_bin(op: Op, left: &Filter, right: &Filter, input: &Value) -> Result<Vec<Value>, Failure> {
    // The right stream is the outer loop. A left error drops the remaining
    // rights; a right error is reported after the successful rights.
    let (rights, right_err) = parts(eval(right, input));
    let mut out = Vec::new();
    for right_value in rights {
        let (lefts, left_err) = parts(eval(left, input));
        for left_value in lefts {
            match apply_op(op, &left_value, &right_value) {
                Ok(value) => out.push(value),
                Err(message) => {
                    return Err(Failure {
                        partial: out,
                        message,
                    })
                }
            }
        }
        if let Some(message) = left_err {
            return Err(Failure {
                partial: out,
                message,
            });
        }
    }
    finish(out, right_err)
}

fn eval_and(left: &Filter, right: &Filter, input: &Value) -> Result<Vec<Value>, Failure> {
    let (lefts, left_err) = parts(eval(left, input));
    let mut out = Vec::new();
    for left_value in lefts {
        if !truthy(&left_value) {
            out.push(Value::Bool(false));
            continue;
        }
        let (rights, right_err) = parts(eval(right, input));
        for right_value in rights {
            out.push(Value::Bool(truthy(&right_value)));
        }
        if let Some(message) = right_err {
            return Err(Failure {
                partial: out,
                message,
            });
        }
    }
    finish(out, left_err)
}

fn eval_or(left: &Filter, right: &Filter, input: &Value) -> Result<Vec<Value>, Failure> {
    let (lefts, left_err) = parts(eval(left, input));
    let mut out = Vec::new();
    for left_value in lefts {
        if truthy(&left_value) {
            out.push(Value::Bool(true));
            continue;
        }
        let (rights, right_err) = parts(eval(right, input));
        for right_value in rights {
            out.push(Value::Bool(truthy(&right_value)));
        }
        if let Some(message) = right_err {
            return Err(Failure {
                partial: out,
                message,
            });
        }
    }
    finish(out, left_err)
}

fn eval_alt(left: &Filter, right: &Filter, input: &Value) -> Result<Vec<Value>, Failure> {
    match eval(left, input) {
        Err(failure) => Err(Failure {
            partial: failure.partial.into_iter().filter(truthy).collect(),
            message: failure.message,
        }),
        Ok(values) => {
            let defined: Vec<Value> = values.into_iter().filter(truthy).collect();
            if defined.is_empty() {
                eval(right, input)
            } else {
                Ok(defined)
            }
        }
    }
}

fn eval_if(cond: &Filter, yes: &Filter, no: &Filter, input: &Value) -> Result<Vec<Value>, Failure> {
    let (conds, err) = parts(eval(cond, input));
    let mut out = Vec::new();
    for cond_value in conds {
        let branch = if truthy(&cond_value) { yes } else { no };
        match eval(branch, input) {
            Ok(values) => out.extend(values),
            Err(failure) => {
                out.extend(failure.partial);
                return Err(Failure {
                    partial: out,
                    message: failure.message,
                });
            }
        }
    }
    finish(out, err)
}

fn apply_op(op: Op, left: &Value, right: &Value) -> Result<Value, String> {
    match op {
        Op::Add => add_values(left, right),
        Op::Sub => sub_values(left, right),
        Op::Mul => mul_values(left, right),
        Op::Div => div_values(left, right),
        Op::Rem => rem_values(left, right),
        Op::Eq => Ok(Value::Bool(cmp_value(left, right) == 0)),
        Op::Ne => Ok(Value::Bool(cmp_value(left, right) != 0)),
        Op::Lt => Ok(Value::Bool(cmp_value(left, right) < 0)),
        Op::Gt => Ok(Value::Bool(cmp_value(left, right) > 0)),
        Op::Le => Ok(Value::Bool(cmp_value(left, right) <= 0)),
        Op::Ge => Ok(Value::Bool(cmp_value(left, right) >= 0)),
    }
}

fn add_values(left: &Value, right: &Value) -> Result<Value, String> {
    match (left, right) {
        (Value::Null, other) | (other, Value::Null) => Ok(other.clone()),
        (Value::Number(a), Value::Number(b)) => Ok(num_binop(a, b, NumOp::Add)),
        (Value::String(a), Value::String(b)) => {
            let mut text = a.clone();
            text.push_str(b);
            Ok(Value::String(text))
        }
        (Value::Array(a), Value::Array(b)) => {
            let mut items = a.clone();
            items.extend(b.iter().cloned());
            Ok(Value::Array(items))
        }
        (Value::Object(a), Value::Object(b)) => Ok(merge_objects(a, b)),
        _ => Err(binary_type(left, right, "cannot be added")),
    }
}

fn sub_values(left: &Value, right: &Value) -> Result<Value, String> {
    match (left, right) {
        (Value::Number(a), Value::Number(b)) => Ok(num_binop(a, b, NumOp::Sub)),
        (Value::Array(a), Value::Array(b)) => Ok(array_sub(a, b)),
        _ => Err(binary_type(left, right, "cannot be subtracted")),
    }
}

fn mul_values(left: &Value, right: &Value) -> Result<Value, String> {
    match (left, right) {
        (Value::Number(a), Value::Number(b)) => Ok(num_binop(a, b, NumOp::Mul)),
        (Value::String(text), Value::Number(n)) | (Value::Number(n), Value::String(text)) => {
            match string_times(text, number::real(n)) {
                Some(value) => Ok(value),
                None => Err(binary_type(left, right, "cannot be multiplied")),
            }
        }
        (Value::Object(a), Value::Object(b)) => Ok(merge_deep(a, b)),
        _ => Err(binary_type(left, right, "cannot be multiplied")),
    }
}

fn div_values(left: &Value, right: &Value) -> Result<Value, String> {
    match (left, right) {
        (Value::Number(a), Value::Number(b)) => num_div(left, right, a, b),
        (Value::String(text), Value::String(sep)) => Ok(split_string(text, sep)),
        _ => Err(binary_type(left, right, "cannot be divided")),
    }
}

fn rem_values(left: &Value, right: &Value) -> Result<Value, String> {
    match (left, right) {
        (Value::Number(a), Value::Number(b)) => num_rem(left, right, a, b),
        _ => Err(binary_type(left, right, "cannot be divided (remainder)")),
    }
}

enum NumOp {
    Add,
    Sub,
    Mul,
}

fn num_binop(left: &str, right: &str, op: NumOp) -> Value {
    let value = match (number::real(left), number::real(right)) {
        (number::Real::Nan, _) | (_, number::Real::Nan) => {
            return Value::Number(number::mark_f64(number::nan_text()))
        }
        (number::Real::Num(a), number::Real::Num(b)) => match op {
            NumOp::Add => a + b,
            NumOp::Sub => a - b,
            NumOp::Mul => a * b,
        },
    };
    Value::Number(number::mark_f64(number::format_jq_f64(value)))
}

fn num_div(left: &Value, right: &Value, a: &str, b: &str) -> Result<Value, String> {
    if let number::Real::Num(divisor) = number::real(b) {
        if divisor == 0.0 {
            return Err(zero_divisor(left, right, false));
        }
    }
    let value = match (number::real(a), number::real(b)) {
        (number::Real::Nan, _) | (_, number::Real::Nan) => {
            return Ok(Value::Number(number::mark_f64(number::nan_text())))
        }
        (number::Real::Num(x), number::Real::Num(y)) => x / y,
    };
    Ok(Value::Number(number::mark_f64(number::format_jq_f64(
        value,
    ))))
}

fn num_rem(left: &Value, right: &Value, a: &str, b: &str) -> Result<Value, String> {
    // A NaN operand wins over a zero divisor. `nan % 0` is NaN.
    if matches!(number::real(a), number::Real::Nan) || matches!(number::real(b), number::Real::Nan)
    {
        return Ok(Value::Number(number::mark_f64(number::nan_text())));
    }
    let number::Real::Num(y) = number::real(b) else {
        return Ok(Value::Number(number::mark_f64(number::nan_text())));
    };
    // Truncate toward zero into jq's integer width. A fractional divisor
    // such as `0.5` is zero after that truncation.
    let divisor = number::jq_dtoi(y);
    if divisor == 0 {
        return Err(zero_divisor(left, right, true));
    }
    let number::Real::Num(x) = number::real(a) else {
        return Ok(Value::Number(number::mark_f64(number::nan_text())));
    };
    // `i64::MIN % -1` does not fit in the quotient. Every integer divides by -1.
    let remainder = if divisor == -1 {
        0.0
    } else {
        (number::jq_dtoi(x) % divisor) as f64
    };
    Ok(Value::Number(number::mark_f64(number::format_jq_f64(
        remainder,
    ))))
}

fn negate_value(value: &Value) -> Result<Value, String> {
    let Value::Number(text) = value else {
        return Err(format!(
            "{} ({}) cannot be negated",
            value.type_name(),
            shown(value)
        ));
    };
    let negated = match number::real(text) {
        number::Real::Nan => return Ok(Value::Number(number::mark_f64(number::nan_text()))),
        number::Real::Num(n) => -n,
    };
    Ok(Value::Number(number::mark_f64(number::format_jq_f64(
        negated,
    ))))
}

/// `None` means the repeat would be enormous. jq builds it; we refuse.
fn string_times(text: &str, n: number::Real) -> Option<Value> {
    let number::Real::Num(n) = n else {
        return Some(Value::Null);
    };
    if n < 0.0 {
        return Some(Value::Null);
    }
    if !n.is_finite() {
        return None;
    }
    let count = n.trunc();
    if count > 8_000_000.0 {
        return None;
    }
    Some(Value::String(text.repeat(count as usize)))
}

fn split_string(text: &str, sep: &str) -> Value {
    if sep.is_empty() {
        return Value::Array(
            text.chars()
                .map(|ch| Value::String(ch.to_string()))
                .collect(),
        );
    }
    if text.is_empty() {
        return Value::Array(Vec::new());
    }
    let mut parts = Vec::new();
    let mut rest = text;
    while let Some(at) = rest.find(sep) {
        parts.push(Value::String(rest[..at].to_string()));
        rest = &rest[at + sep.len()..];
    }
    parts.push(Value::String(rest.to_string()));
    Value::Array(parts)
}

fn array_sub(left: &[Value], right: &[Value]) -> Value {
    let mut out = Vec::new();
    for item in left {
        if !right.iter().any(|other| cmp_value(item, other) == 0) {
            out.push(item.clone());
        }
    }
    Value::Array(out)
}

fn merge_objects(left: &[(String, Value)], right: &[(String, Value)]) -> Value {
    let mut pairs = left.to_vec();
    for (key, value) in right {
        put(&mut pairs, key.clone(), value.clone());
    }
    Value::Object(pairs)
}

fn merge_deep(left: &[(String, Value)], right: &[(String, Value)]) -> Value {
    let mut pairs = Vec::new();
    for (key, left_value) in left {
        if let Some((_, right_value)) = right.iter().find(|(name, _)| name == key) {
            match (left_value, right_value) {
                (Value::Object(left_obj), Value::Object(right_obj)) => {
                    pairs.push((key.clone(), merge_deep(left_obj, right_obj)));
                }
                _ => pairs.push((key.clone(), right_value.clone())),
            }
        } else {
            pairs.push((key.clone(), left_value.clone()));
        }
    }
    for (key, right_value) in right {
        if left.iter().all(|(name, _)| name != key) {
            pairs.push((key.clone(), right_value.clone()));
        }
    }
    Value::Object(pairs)
}

fn type_rank(value: &Value) -> u8 {
    match value {
        Value::Null => 0,
        Value::Bool(false) => 1,
        Value::Bool(true) => 2,
        Value::Number(_) => 3,
        Value::String(_) => 4,
        Value::Array(_) => 5,
        Value::Object(_) => 6,
    }
}

fn cmp_value(left: &Value, right: &Value) -> i8 {
    let left_rank = type_rank(left);
    let right_rank = type_rank(right);
    if left_rank != right_rank {
        return if left_rank < right_rank { -1 } else { 1 };
    }
    match (left, right) {
        (Value::Number(a), Value::Number(b)) => cmp_number(a, b),
        (Value::String(a), Value::String(b)) => ord_i8(a.cmp(b)),
        (Value::Array(a), Value::Array(b)) => cmp_array(a, b),
        (Value::Object(a), Value::Object(b)) => cmp_object(a, b),
        _ => 0,
    }
}

fn cmp_number(left: &str, right: &str) -> i8 {
    number::cmp_number_text(left, right)
}

fn cmp_array(left: &[Value], right: &[Value]) -> i8 {
    for (a, b) in left.iter().zip(right.iter()) {
        let cmp = cmp_value(a, b);
        if cmp != 0 {
            return cmp;
        }
    }
    ord_i8(left.len().cmp(&right.len()))
}

fn cmp_object(left: &[(String, Value)], right: &[(String, Value)]) -> i8 {
    let mut left_keys: Vec<&str> = left.iter().map(|(key, _)| key.as_str()).collect();
    let mut right_keys: Vec<&str> = right.iter().map(|(key, _)| key.as_str()).collect();
    left_keys.sort_unstable();
    right_keys.sort_unstable();
    for (a, b) in left_keys.iter().zip(right_keys.iter()) {
        match a.cmp(b) {
            std::cmp::Ordering::Equal => {}
            std::cmp::Ordering::Less => return -1,
            std::cmp::Ordering::Greater => return 1,
        }
    }
    if left_keys.len() != right_keys.len() {
        return if left_keys.len() < right_keys.len() {
            -1
        } else {
            1
        };
    }
    for key in left_keys {
        let left_value = object_get(left, key);
        let right_value = object_get(right, key);
        let cmp = cmp_value(left_value, right_value);
        if cmp != 0 {
            return cmp;
        }
    }
    0
}

fn object_get<'a>(items: &'a [(String, Value)], key: &str) -> &'a Value {
    items
        .iter()
        .find(|(name, _)| name == key)
        .map(|(_, value)| value)
        .expect("key is present")
}

fn ord_i8(order: std::cmp::Ordering) -> i8 {
    match order {
        std::cmp::Ordering::Less => -1,
        std::cmp::Ordering::Equal => 0,
        std::cmp::Ordering::Greater => 1,
    }
}

fn binary_type(left: &Value, right: &Value, verb: &str) -> String {
    format!(
        "{} ({}) and {} ({}) {verb}",
        left.type_name(),
        shown(left),
        right.type_name(),
        shown(right)
    )
}

fn zero_divisor(left: &Value, right: &Value, remainder: bool) -> String {
    let verb = if remainder {
        "cannot be divided (remainder)"
    } else {
        "cannot be divided"
    };
    format!(
        "{} because the divisor is zero",
        binary_type(left, right, verb)
    )
}

struct Parser<'a> {
    src: &'a str,
    index: usize,
}

impl<'a> Parser<'a> {
    fn parse_pipe(&mut self) -> Result<Filter, ProgramError> {
        let mut filter = self.parse_comma()?;
        while self.eat('|') {
            let right = self.parse_comma()?;
            filter = Filter::Pipe(Box::new(filter), Box::new(right));
        }
        Ok(filter)
    }

    fn parse_comma(&mut self) -> Result<Filter, ProgramError> {
        let mut filter = self.parse_alt()?;
        while self.eat(',') {
            let right = self.parse_alt()?;
            filter = Filter::Comma(Box::new(filter), Box::new(right));
        }
        Ok(filter)
    }

    /// `//` is right-associative and looser than `or`.
    fn parse_alt(&mut self) -> Result<Filter, ProgramError> {
        let left = self.parse_or()?;
        if self.eat_alt() {
            let right = self.parse_alt()?;
            return Ok(Filter::Alt(Box::new(left), Box::new(right)));
        }
        Ok(left)
    }

    fn parse_or(&mut self) -> Result<Filter, ProgramError> {
        let mut left = self.parse_and()?;
        while self.eat_keyword("or") {
            let right = self.parse_and()?;
            left = Filter::Or(Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn parse_and(&mut self) -> Result<Filter, ProgramError> {
        let mut left = self.parse_cmp()?;
        while self.eat_keyword("and") {
            let right = self.parse_cmp()?;
            left = Filter::And(Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    /// Comparisons do not chain. `1<2<3` is a syntax error.
    fn parse_cmp(&mut self) -> Result<Filter, ProgramError> {
        let left = self.parse_add()?;
        let Some(op) = self.cmp_op() else {
            return Ok(left);
        };
        let right = self.parse_add()?;
        if self.cmp_op_pending() {
            return Err(ProgramError::Syntax);
        }
        Ok(Filter::Bin {
            op,
            left: Box::new(left),
            right: Box::new(right),
        })
    }

    fn parse_add(&mut self) -> Result<Filter, ProgramError> {
        let mut left = self.parse_signed()?;
        loop {
            self.skip_ws();
            let op = match self.peek() {
                Some('+') => Op::Add,
                Some('-') => Op::Sub,
                _ => break,
            };
            self.bump();
            let right = self.parse_signed()?;
            left = Filter::Bin {
                op,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    /// A leading `-` binds looser than `*` `/` `%`, so `-1/2` is `-(1/2)`.
    fn parse_signed(&mut self) -> Result<Filter, ProgramError> {
        self.skip_ws();
        if self.peek() == Some('-') {
            self.bump();
            let operand = self.parse_signed()?;
            return Ok(Filter::Neg(Box::new(operand)));
        }
        self.parse_mul()
    }

    fn parse_mul(&mut self) -> Result<Filter, ProgramError> {
        let mut left = self.parse_juxt()?;
        while let Some(op) = self.mul_op() {
            // A minus here is the start of the operand, and that operand
            // includes the tighter `*` `/` `%` that follow it: `2/-3*4`
            // is `2/-(3*4)`. Without the minus, `8/2/2` stays left-associative.
            let right = if self.sees('-') {
                self.parse_signed()?
            } else {
                self.parse_juxt()?
            };
            left = Filter::Bin {
                op,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    fn parse_juxt(&mut self) -> Result<Filter, ProgramError> {
        let mut filter = self.parse_postfix()?;
        loop {
            self.skip_ws();
            if matches!(self.peek(), Some('.' | '(')) {
                // `not(1)` is a function call. Calls are not in this version.
                if self.peek() == Some('(') && matches!(filter, Filter::Not) {
                    return Err(ProgramError::Syntax);
                }
                let right = self.parse_postfix()?;
                filter = Filter::Pipe(Box::new(filter), Box::new(right));
            } else {
                break;
            }
        }
        Ok(filter)
    }

    fn parse_postfix(&mut self) -> Result<Filter, ProgramError> {
        let mut filter = self.parse_primary()?;
        loop {
            self.skip_ws();
            if self.eat('?') {
                filter = Filter::Optional(Box::new(filter));
                continue;
            }
            if self.peek() == Some('[') {
                filter = self.parse_index(Some(filter))?;
                continue;
            }
            if self.peek() == Some('.') {
                let bytes = self.src.as_bytes();
                // `..` is recurse, and only when it is the whole term.
                if bytes.get(self.index + 1) == Some(&b'.') {
                    break;
                }
                self.bump();
                if self.peek() == Some('"') || self.spaced_quote() {
                    let key = self.parse_quoted_key()?;
                    filter = at_key(Some(filter), key);
                    continue;
                }
                if self.starts_ident() {
                    let name = self.ident()?;
                    filter = chain(Some(filter), Filter::Field(name));
                    continue;
                }
                if self.peek() == Some('[') {
                    filter = self.parse_index(Some(filter))?;
                    continue;
                }
                return Err(ProgramError::Syntax);
            }
            break;
        }
        Ok(filter)
    }

    fn parse_primary(&mut self) -> Result<Filter, ProgramError> {
        self.skip_ws();
        if self.eat('(') {
            let inner = self.parse_pipe()?;
            self.skip_ws();
            if !self.eat(')') {
                return Err(ProgramError::Syntax);
            }
            return Ok(inner);
        }
        if self.peek() == Some('[') {
            return self.parse_array();
        }
        if self.peek() == Some('{') {
            return self.parse_object();
        }
        if self.peek() == Some('"') {
            return self.parse_string_expr();
        }
        if self.peek() == Some('.') {
            let bytes = self.src.as_bytes();
            if bytes
                .get(self.index + 1)
                .is_some_and(|b| b.is_ascii_digit())
            {
                return self.parse_number();
            }
            return self.parse_path();
        }
        if self.peek().is_some_and(|c| c.is_ascii_digit()) {
            return self.parse_number();
        }
        if self.starts_ident() {
            let name = self.ident()?;
            return match name.as_str() {
                "true" => Ok(Filter::Literal(Value::Bool(true))),
                "false" => Ok(Filter::Literal(Value::Bool(false))),
                "null" => Ok(Filter::Literal(Value::Null)),
                "nan" => Ok(Filter::Literal(Value::Number(number::nan_text()))),
                "infinite" => Ok(Filter::Literal(Value::Number(number::pos_inf()))),
                "not" => Ok(Filter::Not),
                "if" => self.parse_if(),
                _ => Err(ProgramError::Syntax),
            };
        }
        Err(ProgramError::Syntax)
    }

    /// Pipe and unary minus, without comma or the arithmetic operators.
    /// Object values use this, so `{a:1+2}` stays a syntax error.
    fn parse_expd(&mut self) -> Result<Filter, ProgramError> {
        let mut filter = self.parse_expd_unary()?;
        while self.eat('|') {
            let right = self.parse_expd_unary()?;
            filter = Filter::Pipe(Box::new(filter), Box::new(right));
        }
        Ok(filter)
    }

    fn parse_expd_unary(&mut self) -> Result<Filter, ProgramError> {
        self.skip_ws();
        if self.peek() == Some('-') {
            self.bump();
            let operand = self.parse_expd_unary()?;
            return Ok(Filter::Neg(Box::new(operand)));
        }
        self.parse_postfix()
    }

    fn parse_if(&mut self) -> Result<Filter, ProgramError> {
        let cond = self.parse_pipe()?;
        if !self.eat_keyword("then") {
            return Err(ProgramError::Syntax);
        }
        let yes = self.parse_pipe()?;
        let no = self.parse_else()?;
        Ok(Filter::If {
            cond: Box::new(cond),
            yes: Box::new(yes),
            no: Box::new(no),
        })
    }

    fn parse_else(&mut self) -> Result<Filter, ProgramError> {
        if self.eat_keyword("elif") {
            let cond = self.parse_pipe()?;
            if !self.eat_keyword("then") {
                return Err(ProgramError::Syntax);
            }
            let yes = self.parse_pipe()?;
            let no = self.parse_else()?;
            return Ok(Filter::If {
                cond: Box::new(cond),
                yes: Box::new(yes),
                no: Box::new(no),
            });
        }
        let no = if self.eat_keyword("else") {
            self.parse_pipe()?
        } else {
            // A missing else yields the input, which is null under -n.
            Filter::Identity
        };
        if !self.eat_keyword("end") {
            return Err(ProgramError::Syntax);
        }
        Ok(no)
    }

    fn eat_keyword(&mut self, word: &str) -> bool {
        self.skip_ws();
        let rest = &self.src[self.index..];
        if rest.len() < word.len() || !rest.starts_with(word) {
            return false;
        }
        let boundary = match rest[word.len()..].chars().next() {
            None => true,
            Some(ch) if ch.is_ascii_alphanumeric() || ch == '_' => false,
            Some(_) => true,
        };
        if !boundary {
            return false;
        }
        self.index += word.len();
        true
    }

    fn eat_alt(&mut self) -> bool {
        self.skip_ws();
        if self.src[self.index..].starts_with("//") {
            self.index += 2;
            true
        } else {
            false
        }
    }

    fn sees(&mut self, expected: char) -> bool {
        self.skip_ws();
        self.peek() == Some(expected)
    }

    fn cmp_op(&mut self) -> Option<Op> {
        self.skip_ws();
        let (op, len) = cmp_at(&self.src[self.index..])?;
        self.index += len;
        Some(op)
    }

    fn cmp_op_pending(&mut self) -> bool {
        self.skip_ws();
        cmp_at(&self.src[self.index..]).is_some()
    }

    /// `*` `/` `%`, but not `//`.
    fn mul_op(&mut self) -> Option<Op> {
        self.skip_ws();
        let rest = &self.src[self.index..];
        if rest.starts_with("//") {
            return None;
        }
        let (op, len) = if rest.starts_with('*') {
            (Op::Mul, 1)
        } else if rest.starts_with('/') {
            (Op::Div, 1)
        } else if rest.starts_with('%') {
            (Op::Rem, 1)
        } else {
            return None;
        };
        self.index += len;
        Some(op)
    }

    fn parse_path(&mut self) -> Result<Filter, ProgramError> {
        self.bump();
        if self.peek() == Some('.') {
            self.bump();
            if self.peek() == Some('.') || self.starts_ident() {
                return Err(ProgramError::Syntax);
            }
            return self.finish_path(Some(Filter::Recurse));
        }
        let mut filter = None;
        if self.peek() == Some('"') || self.spaced_quote() {
            let key = self.parse_quoted_key()?;
            filter = Some(at_key(None, key));
        } else if self.starts_ident() {
            filter = Some(Filter::Field(self.ident()?));
        }
        self.finish_path(filter)
    }

    fn finish_path(&mut self, mut filter: Option<Filter>) -> Result<Filter, ProgramError> {
        loop {
            self.skip_ws();
            if self.eat('?') {
                let base = filter.unwrap_or(Filter::Identity);
                filter = Some(optional_last(base));
                continue;
            }
            self.skip_ws();
            if self.peek() == Some('[') {
                filter = Some(self.parse_index(filter)?);
                continue;
            }
            if self.peek() == Some('.') {
                let bytes = self.src.as_bytes();
                if bytes.get(self.index + 1) == Some(&b'.') {
                    return Err(ProgramError::Syntax);
                }
                self.bump();
                if self.peek() == Some('"') || self.spaced_quote() {
                    let key = self.parse_quoted_key()?;
                    filter = Some(at_key(filter, key));
                    continue;
                }
                if self.starts_ident() {
                    let name = self.ident()?;
                    filter = Some(chain(filter, Filter::Field(name)));
                    continue;
                }
                if self.peek() == Some('[') {
                    filter = Some(self.parse_index(filter)?);
                    continue;
                }
                return Err(ProgramError::Syntax);
            }
            break;
        }
        Ok(filter.unwrap_or(Filter::Identity))
    }

    /// A space is allowed between `.` and a quoted name.
    fn spaced_quote(&self) -> bool {
        let mut index = self.index;
        let bytes = self.src.as_bytes();
        while index < bytes.len() && matches!(bytes[index], b' ' | b'\t' | b'\n' | b'\r') {
            index += 1;
        }
        bytes.get(index) == Some(&b'"')
    }

    fn parse_quoted_key(&mut self) -> Result<Filter, ProgramError> {
        if self.peek() != Some('"') {
            self.skip_ws();
        }
        self.parse_string_expr()
    }

    /// `[` has not been consumed. Slice forms stay on the old path.
    fn parse_index(&mut self, base: Option<Filter>) -> Result<Filter, ProgramError> {
        if !self.eat('[') {
            return Err(ProgramError::Syntax);
        }
        self.skip_ws();
        if self.eat(']') {
            return Ok(chain(base, Filter::Iterate));
        }
        if self.bracket_has_slice() {
            let segment = self.parse_slice_body()?;
            return Ok(chain(base, segment));
        }
        let key = self.parse_pipe()?;
        self.skip_ws();
        if !self.eat(']') {
            return Err(ProgramError::Syntax);
        }
        Ok(at_key(base, key))
    }

    fn bracket_has_slice(&self) -> bool {
        let bytes = self.src.as_bytes();
        let mut index = self.index;
        let mut depth = 0i32;
        let mut in_string = false;
        let mut escape = false;
        while index < bytes.len() {
            let byte = bytes[index];
            if in_string {
                if escape {
                    escape = false;
                } else if byte == b'\\' {
                    escape = true;
                } else if byte == b'"' {
                    in_string = false;
                }
                index += 1;
                continue;
            }
            match byte {
                b'"' => in_string = true,
                b'[' | b'(' | b'{' => depth += 1,
                b']' if depth == 0 => return false,
                b']' | b')' | b'}' => depth -= 1,
                b':' if depth == 0 => return true,
                _ => {}
            }
            index += 1;
        }
        false
    }

    /// The opening `[` is already consumed, and the body contains a `:`.
    fn parse_slice_body(&mut self) -> Result<Filter, ProgramError> {
        let mut toks = Vec::new();
        loop {
            self.skip_ws();
            if self
                .peek()
                .is_some_and(|c| c == '-' || c == '.' || c.is_ascii_digit())
            {
                toks.push(BracketTok::Num(self.number_literal()?.to_string()));
            }
            self.skip_ws();
            if self.eat(':') {
                toks.push(BracketTok::Colon);
                continue;
            }
            if self.peek() == Some(',') {
                self.bump();
                toks.push(BracketTok::Comma);
                continue;
            }
            if self.eat(']') {
                break;
            }
            return Err(ProgramError::Syntax);
        }
        filter_from_brackets(toks)
    }

    fn number_literal(&mut self) -> Result<&'a str, ProgramError> {
        let start = self.index;
        if self.peek() == Some('-') {
            self.bump();
        }
        if self.peek() == Some('.') {
            self.bump();
            if !self.take_digits() {
                return Err(ProgramError::Syntax);
            }
        } else if self.peek().is_some_and(|c| c.is_ascii_digit()) {
            self.take_digits();
            if self.peek() == Some('.') {
                self.bump();
                self.take_digits();
            }
        } else {
            return Err(ProgramError::Syntax);
        }
        if matches!(self.peek(), Some('e' | 'E')) {
            self.bump();
            if matches!(self.peek(), Some('+' | '-')) {
                self.bump();
            }
            if !self.take_digits() {
                return Err(ProgramError::Syntax);
            }
        }
        Ok(&self.src[start..self.index])
    }

    fn take_digits(&mut self) -> bool {
        let mut any = false;
        while self.peek().is_some_and(|c| c.is_ascii_digit()) {
            any = true;
            self.bump();
        }
        any
    }

    fn ident(&mut self) -> Result<String, ProgramError> {
        let start = self.index;
        if !self.starts_ident() {
            return Err(ProgramError::Syntax);
        }
        self.bump();
        while self
            .peek()
            .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_')
        {
            self.bump();
        }
        Ok(self.src[start..self.index].to_string())
    }

    fn starts_ident(&self) -> bool {
        matches!(self.peek(), Some(c) if c.is_ascii_alphabetic() || c == '_')
    }

    fn eat(&mut self, expected: char) -> bool {
        self.skip_ws();
        if self.peek() == Some(expected) {
            self.bump();
            true
        } else {
            false
        }
    }

    fn skip_ws(&mut self) {
        loop {
            match self.peek() {
                Some(' ' | '\t' | '\n' | '\r') => {
                    self.bump();
                }
                Some('#') => {
                    self.bump();
                    while self.peek().is_some_and(|c| c != '\n') {
                        self.bump();
                    }
                }
                _ => break,
            }
        }
    }

    fn parse_number(&mut self) -> Result<Filter, ProgramError> {
        let start = self.index;
        let bytes = self.src.as_bytes();
        let mut index = start;
        if bytes.get(index) == Some(&b'.') {
            index += 1;
            let frac = index;
            while index < bytes.len() && bytes[index].is_ascii_digit() {
                index += 1;
            }
            if index == frac {
                return Err(ProgramError::Syntax);
            }
        } else {
            while index < bytes.len() && bytes[index].is_ascii_digit() {
                index += 1;
            }
            if bytes.get(index) == Some(&b'.') {
                index += 1;
                while index < bytes.len() && bytes[index].is_ascii_digit() {
                    index += 1;
                }
            }
        }
        if index < bytes.len() && (bytes[index] == b'e' || bytes[index] == b'E') {
            let exp = index + 1;
            let mut digit_at = exp;
            if bytes
                .get(digit_at)
                .is_some_and(|b| *b == b'+' || *b == b'-')
            {
                digit_at += 1;
            }
            if bytes.get(digit_at).is_some_and(|b| b.is_ascii_digit()) {
                index = digit_at + 1;
                while index < bytes.len() && bytes[index].is_ascii_digit() {
                    index += 1;
                }
            }
        }
        // A leading dot with no digits is a path, not a number.
        let token = &self.src[start..index];
        if token == "." || token.is_empty() {
            return Err(ProgramError::Syntax);
        }
        self.index = index;
        let value = number_value(token)?;
        Ok(Filter::Literal(value))
    }

    fn parse_array(&mut self) -> Result<Filter, ProgramError> {
        if !self.eat('[') {
            return Err(ProgramError::Syntax);
        }
        self.skip_ws();
        if self.eat(']') {
            return Ok(Filter::Literal(Value::Array(Vec::new())));
        }
        let inner = self.parse_pipe()?;
        self.skip_ws();
        if !self.eat(']') {
            return Err(ProgramError::Syntax);
        }
        if let Some(items) = const_outputs(&inner) {
            return Ok(Filter::Literal(Value::Array(items)));
        }
        Ok(Filter::Collect(Box::new(inner)))
    }

    fn parse_object(&mut self) -> Result<Filter, ProgramError> {
        if !self.eat('{') {
            return Err(ProgramError::Syntax);
        }
        self.skip_ws();
        if self.eat('}') {
            return Ok(Filter::Literal(Value::Object(Vec::new())));
        }
        let mut fields = Vec::new();
        loop {
            fields.push(self.parse_field()?);
            self.skip_ws();
            if self.eat(',') {
                self.skip_ws();
                if self.eat('}') {
                    break;
                }
                continue;
            }
            if self.eat('}') {
                break;
            }
            return Err(ProgramError::Syntax);
        }
        if let Some(value) = const_object(&fields) {
            return Ok(Filter::Literal(value));
        }
        Ok(Filter::Object(fields))
    }

    fn parse_field(&mut self) -> Result<ObjField, ProgramError> {
        self.skip_ws();
        if self.peek() == Some('(') {
            let pos = self.index;
            self.bump();
            let key = self.parse_pipe()?;
            self.skip_ws();
            if !self.eat(')') || !self.eat(':') {
                return Err(ProgramError::Syntax);
            }
            let value = self.parse_expd()?;
            if let Some(constant) = const_one(&key) {
                if !matches!(constant, Value::String(_)) {
                    return Err(report_at(self.src, pos, &key_message(&constant)));
                }
            }
            return Ok(ObjField::Pair {
                key: Box::new(key),
                value: Box::new(value),
            });
        }
        if self.peek() == Some('"') {
            let key = self.parse_string_expr()?;
            self.skip_ws();
            if self.eat(':') {
                let value = self.parse_expd()?;
                return Ok(ObjField::Pair {
                    key: Box::new(key),
                    value: Box::new(value),
                });
            }
            return Ok(ObjField::Lookup(Box::new(key)));
        }
        if self.starts_ident() {
            let name = self.ident_key()?;
            self.skip_ws();
            if self.eat(':') {
                let value = self.parse_expd()?;
                return Ok(ObjField::Pair {
                    key: Box::new(Filter::Literal(Value::String(name))),
                    value: Box::new(value),
                });
            }
            return Ok(ObjField::Pair {
                key: Box::new(Filter::Literal(Value::String(name.clone()))),
                value: Box::new(Filter::Field(name)),
            });
        }
        Err(ProgramError::Syntax)
    }

    /// `foo::bar` is one key.
    fn ident_key(&mut self) -> Result<String, ProgramError> {
        let mut name = self.ident()?;
        while self.src[self.index..].starts_with("::") {
            let after = self.index + 2;
            if after >= self.src.len() {
                return Err(ProgramError::Syntax);
            }
            let saved = self.index;
            self.index = after;
            if !self.starts_ident() {
                self.index = saved;
                break;
            }
            name.push_str("::");
            name.push_str(&self.ident()?);
        }
        Ok(name)
    }

    fn parse_string_expr(&mut self) -> Result<Filter, ProgramError> {
        if !self.eat('"') {
            return Err(ProgramError::Syntax);
        }
        let mut pieces = Vec::new();
        let mut text = String::new();
        loop {
            let Some(ch) = self.peek() else {
                return Err(ProgramError::Syntax);
            };
            if ch == '"' {
                self.bump();
                break;
            }
            if ch == '\\' {
                let slash = self.index;
                let next = self.src[self.index + 1..].chars().next();
                if next == Some('(') {
                    if !text.is_empty() {
                        pieces.push(Piece::Text(std::mem::take(&mut text)));
                    }
                    self.bump();
                    self.bump();
                    let inner = self.parse_pipe()?;
                    if !self.eat(')') {
                        return Err(ProgramError::Syntax);
                    }
                    pieces.push(Piece::Interp(Box::new(inner)));
                    continue;
                }
                if !self.take_escape_run() {
                    return Err(ProgramError::Syntax);
                }
                let run = &self.src[slash..self.index];
                let wrapped = format!("\"{run}\"");
                match decode_string_token(&wrapped) {
                    Ok(decoded) => text.push_str(&decoded),
                    Err(message) => return Err(report_at(self.src, slash, &message)),
                }
                continue;
            }
            self.bump();
            text.push(ch);
        }
        if !text.is_empty() {
            pieces.push(Piece::Text(text));
        }
        if pieces.iter().all(|piece| matches!(piece, Piece::Text(_))) {
            let mut combined = String::new();
            for piece in pieces {
                if let Piece::Text(part) = piece {
                    combined.push_str(&part);
                }
            }
            return Ok(Filter::Literal(Value::String(combined)));
        }
        Ok(Filter::Interpolate(pieces))
    }

    /// One or more jq escape chunks, starting at a backslash.
    fn take_escape_run(&mut self) -> bool {
        let bytes = self.src.as_bytes();
        let mut index = self.index;
        let mut any = false;
        while index < bytes.len() && bytes[index] == b'\\' {
            if index + 1 >= bytes.len() {
                break;
            }
            let next = bytes[index + 1];
            if next == b'(' {
                break;
            }
            if next == b'u' {
                let mut cursor = index + 2;
                let mut count = 0;
                while count < 4 && cursor < bytes.len() && bytes[cursor].is_ascii_alphanumeric() {
                    cursor += 1;
                    count += 1;
                }
                index = cursor;
                any = true;
                continue;
            }
            index += 2;
            any = true;
        }
        if !any {
            return false;
        }
        self.index = index;
        true
    }

    fn peek(&self) -> Option<char> {
        self.src[self.index..].chars().next()
    }

    fn bump(&mut self) {
        if let Some(c) = self.peek() {
            self.index += c.len_utf8();
        }
    }

    fn eof(&self) -> bool {
        self.index >= self.src.len()
    }
}

enum BracketTok {
    Num(String),
    Colon,
    Comma,
}

fn filter_from_brackets(toks: Vec<BracketTok>) -> Result<Filter, ProgramError> {
    let colons = toks
        .iter()
        .filter(|tok| matches!(tok, BracketTok::Colon))
        .count();
    if colons > 1 {
        return Err(ProgramError::Syntax);
    }
    if colons == 1 {
        let at = toks
            .iter()
            .position(|tok| matches!(tok, BracketTok::Colon))
            .expect("colon");
        let starts = bracket_side(&toks[..at])?;
        let ends = bracket_side(&toks[at + 1..])?;
        if starts.iter().all(Option::is_none) && ends.iter().all(Option::is_none) {
            return Err(ProgramError::Syntax);
        }
        let mut filters = Vec::new();
        for start in &starts {
            for end in &ends {
                filters.push(Filter::Slice {
                    start: start.clone(),
                    end: end.clone(),
                });
            }
        }
        return Ok(combine_commas(filters));
    }
    let mut items = Vec::new();
    let mut expect_value = true;
    for tok in toks {
        match tok {
            BracketTok::Comma if !expect_value => expect_value = true,
            BracketTok::Num(literal) if expect_value => {
                let index = match array_index(&literal) {
                    Some(index) => ArrayIndex::At(index),
                    None => ArrayIndex::OutOfRange,
                };
                items.push(Filter::Index(index));
                expect_value = false;
            }
            _ => return Err(ProgramError::Syntax),
        }
    }
    if expect_value || items.is_empty() {
        return Err(ProgramError::Syntax);
    }
    Ok(combine_commas(items))
}

fn bracket_side(toks: &[BracketTok]) -> Result<Vec<Option<String>>, ProgramError> {
    if toks.is_empty() {
        return Ok(vec![None]);
    }
    let mut bounds = Vec::new();
    let mut expect_value = true;
    for tok in toks {
        match tok {
            BracketTok::Comma if !expect_value => expect_value = true,
            BracketTok::Num(literal) if expect_value => {
                bounds.push(Some(literal.clone()));
                expect_value = false;
            }
            _ => return Err(ProgramError::Syntax),
        }
    }
    if expect_value || bounds.is_empty() {
        return Err(ProgramError::Syntax);
    }
    Ok(bounds)
}

fn combine_commas(items: Vec<Filter>) -> Filter {
    let mut items = items.into_iter();
    let mut filter = items.next().expect("one bracket term");
    for item in items {
        filter = Filter::Comma(Box::new(filter), Box::new(item));
    }
    filter
}

fn chain(left: Option<Filter>, right: Filter) -> Filter {
    match left {
        Some(left) => Filter::Pipe(Box::new(left), Box::new(right)),
        None => right,
    }
}

fn optional_last(filter: Filter) -> Filter {
    match filter {
        Filter::Pipe(left, right) => Filter::Pipe(left, Box::new(Filter::Optional(right))),
        other => Filter::Optional(Box::new(other)),
    }
}

fn cmp_at(rest: &str) -> Option<(Op, usize)> {
    if rest.starts_with("==") {
        Some((Op::Eq, 2))
    } else if rest.starts_with("!=") {
        Some((Op::Ne, 2))
    } else if rest.starts_with("<=") {
        Some((Op::Le, 2))
    } else if rest.starts_with(">=") {
        Some((Op::Ge, 2))
    } else if rest.starts_with('<') {
        Some((Op::Lt, 1))
    } else if rest.starts_with('>') {
        Some((Op::Gt, 1))
    } else {
        None
    }
}

fn at_key(base: Option<Filter>, key: Filter) -> Filter {
    if let Filter::Literal(Value::String(name)) = key {
        return chain(base, Filter::Field(name));
    }
    Filter::At {
        base: Box::new(base.unwrap_or(Filter::Identity)),
        key: Box::new(key),
    }
}

fn number_value(token: &str) -> Result<Value, ProgramError> {
    let parsed = crate::parse_stream(token.as_bytes()).map_err(|_| ProgramError::Syntax)?;
    if parsed.error.is_some() {
        return Err(ProgramError::Syntax);
    }
    match parsed.values.as_slice() {
        [one] => Ok(one.value.clone()),
        _ => Err(ProgramError::Syntax),
    }
}

fn decode_string_token(text: &str) -> Result<String, String> {
    crate::parse::decode_string_token(text)
}

fn report_at(source: &str, pos: usize, message: &str) -> ProgramError {
    let (line_no, line, column) = line_at(source, pos);
    let mut text = format!("jq: error: {message} at <top-level>, line {line_no}:\n{line}");
    text.push_str(&" ".repeat(column));
    text.push_str("\njq: 1 compile error\n");
    ProgramError::Report(text)
}

fn line_at(source: &str, pos: usize) -> (usize, String, usize) {
    let mut line_no = 1usize;
    let mut start = 0usize;
    for (index, byte) in source.bytes().enumerate() {
        if index == pos {
            break;
        }
        if byte == b'\n' {
            line_no += 1;
            start = index + 1;
        }
    }
    let end = source[start..]
        .bytes()
        .position(|byte| byte == b'\n')
        .map(|offset| start + offset)
        .unwrap_or(source.len());
    (
        line_no,
        source[start..end].to_string(),
        pos.saturating_sub(start),
    )
}

fn key_message(value: &Value) -> String {
    format!(
        "Cannot use {} ({}) as object key",
        value.type_name(),
        shown(value)
    )
}

fn const_one(filter: &Filter) -> Option<Value> {
    match const_outputs(filter)?.as_slice() {
        [one] => Some(one.clone()),
        _ => None,
    }
}

fn const_outputs(filter: &Filter) -> Option<Vec<Value>> {
    match filter {
        Filter::Literal(value) => Some(vec![value.clone()]),
        Filter::Pipe(left, right) if is_identity(right) => const_outputs(left),
        Filter::Pipe(left, right) if is_identity(left) => const_outputs(right),
        Filter::Comma(left, right) => {
            let mut out = const_outputs(left)?;
            out.extend(const_outputs(right)?);
            Some(out)
        }
        Filter::Collect(inner) => {
            let items = const_outputs(inner)?;
            Some(vec![Value::Array(items)])
        }
        Filter::Object(fields) => const_object(fields).map(|value| vec![value]),
        Filter::Optional(inner) => const_outputs(inner),
        Filter::Bin { op, left, right } => {
            let lefts = const_outputs(left)?;
            let rights = const_outputs(right)?;
            let mut out = Vec::new();
            for right in &rights {
                for left in &lefts {
                    out.push(apply_op(*op, left, right).ok()?);
                }
            }
            Some(out)
        }
        _ => None,
    }
}

fn is_identity(filter: &Filter) -> bool {
    matches!(filter, Filter::Identity)
}

fn const_object(fields: &[ObjField]) -> Option<Value> {
    let mut pairs = Vec::new();
    for field in fields {
        let ObjField::Pair { key, value } = field else {
            return None;
        };
        let Value::String(name) = const_one(key)? else {
            return None;
        };
        let value = const_one(value)?;
        put(&mut pairs, name, value);
    }
    Some(Value::Object(pairs))
}

fn put(pairs: &mut Vec<(String, Value)>, key: String, value: Value) {
    if let Some((_, slot)) = pairs.iter_mut().find(|(name, _)| name == &key) {
        *slot = value;
    } else {
        pairs.push((key, value));
    }
}

fn eval_interpolate(pieces: &[Piece], input: &Value) -> Result<Vec<Value>, Failure> {
    let strings = expand_pieces(pieces, 0, String::new(), input)?;
    Ok(strings.into_iter().map(Value::String).collect())
}

fn expand_pieces(
    pieces: &[Piece],
    index: usize,
    prefix: String,
    input: &Value,
) -> Result<Vec<String>, Failure> {
    let Some(piece) = pieces.get(index) else {
        return Ok(vec![prefix]);
    };
    match piece {
        Piece::Text(text) => {
            let mut next = prefix;
            next.push_str(text);
            expand_pieces(pieces, index + 1, next, input)
        }
        Piece::Interp(filter) => match eval(filter, input) {
            Ok(values) => {
                let mut out = Vec::new();
                for value in values {
                    let mut next = prefix.clone();
                    next.push_str(&jq_text(&value));
                    out.extend(expand_pieces(pieces, index + 1, next, input)?);
                }
                Ok(out)
            }
            Err(failure) => {
                let mut partial = Vec::new();
                for value in failure.partial {
                    let mut next = prefix.clone();
                    next.push_str(&jq_text(&value));
                    match expand_pieces(pieces, index + 1, next, input) {
                        Ok(more) => partial.extend(more),
                        Err(inner) => {
                            for value in inner.partial {
                                if let Value::String(text) = value {
                                    partial.push(text);
                                }
                            }
                            return Err(Failure {
                                partial: partial.into_iter().map(Value::String).collect(),
                                message: failure.message,
                            });
                        }
                    }
                }
                Err(Failure {
                    partial: partial.into_iter().map(Value::String).collect(),
                    message: failure.message,
                })
            }
        },
    }
}

fn jq_text(value: &Value) -> String {
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

fn eval_object(fields: &[ObjField], input: &Value) -> Result<Vec<Value>, Failure> {
    let mut streams = Vec::with_capacity(fields.len());
    for field in fields {
        streams.push(field_alts(field, input));
    }
    combine_fields(&streams, 0, Vec::new())
}

struct Alts {
    pairs: Vec<(String, Value)>,
    error: Option<String>,
}

fn field_alts(field: &ObjField, input: &Value) -> Alts {
    match field {
        ObjField::Lookup(key) => lookup_alts(key, input),
        ObjField::Pair { key, value } => pair_alts(key, value, input),
    }
}

fn lookup_alts(key: &Filter, input: &Value) -> Alts {
    let found = match eval(key, input) {
        Ok(values) => (values, None),
        Err(failure) => (failure.partial, Some(failure.message)),
    };
    let mut pairs = Vec::new();
    for value in found.0 {
        let Value::String(name) = value else {
            return Alts {
                pairs,
                error: Some(key_message(&value)),
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
                    error: Some(failure.message),
                };
            }
        }
    }
    Alts {
        pairs,
        error: found.1,
    }
}

fn pair_alts(key: &Filter, value: &Filter, input: &Value) -> Alts {
    let keys = match eval(key, input) {
        Ok(values) => (values, None),
        Err(failure) => (failure.partial, Some(failure.message)),
    };
    let values = match eval(value, input) {
        Ok(values) => (values, None),
        Err(failure) => (failure.partial, Some(failure.message)),
    };
    let mut pairs = Vec::new();
    for key_value in &keys.0 {
        let Value::String(name) = key_value else {
            return Alts {
                pairs,
                error: Some(key_message(key_value)),
            };
        };
        for item in &values.0 {
            pairs.push((name.clone(), item.clone()));
        }
        if let Some(message) = &values.1 {
            return Alts {
                pairs,
                error: Some(message.clone()),
            };
        }
    }
    Alts {
        pairs,
        error: keys.1.or(values.1),
    }
}

fn combine_fields(
    streams: &[Alts],
    index: usize,
    base: Vec<(String, Value)>,
) -> Result<Vec<Value>, Failure> {
    let Some(stream) = streams.get(index) else {
        return Ok(vec![Value::Object(base)]);
    };
    let mut out = Vec::new();
    for (key, value) in &stream.pairs {
        let mut next = base.clone();
        put(&mut next, key.clone(), value.clone());
        match combine_fields(streams, index + 1, next) {
            Ok(more) => out.extend(more),
            Err(failure) => {
                out.extend(failure.partial);
                return Err(Failure {
                    partial: out,
                    message: failure.message,
                });
            }
        }
    }
    if let Some(message) = &stream.error {
        return Err(Failure {
            partial: out,
            message: message.clone(),
        });
    }
    Ok(out)
}

fn eval_at(base: &Filter, key: &Filter, input: &Value) -> Result<Vec<Value>, Failure> {
    let values = eval(base, input)?;
    let keys = match eval(key, input) {
        Ok(keys) => keys,
        Err(failure) => {
            let mut partial = Vec::new();
            for key in &failure.partial {
                for value in &values {
                    match index_by(value, key) {
                        Ok(got) => partial.extend(got),
                        Err(inner) => {
                            partial.extend(inner.partial);
                            return Err(Failure {
                                partial,
                                message: failure.message,
                            });
                        }
                    }
                }
            }
            return Err(Failure {
                partial,
                message: failure.message,
            });
        }
    };
    let mut out = Vec::new();
    for key in &keys {
        for value in &values {
            match index_by(value, key) {
                Ok(got) => out.extend(got),
                Err(failure) => {
                    out.extend(failure.partial);
                    return Err(Failure {
                        partial: out,
                        message: failure.message,
                    });
                }
            }
        }
    }
    Ok(out)
}

fn index_by(base: &Value, key: &Value) -> Result<Vec<Value>, Failure> {
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
        Value::Array(_) if matches!(base, Value::Array(_)) => Ok(vec![Value::Array(Vec::new())]),
        other => Err(fail(format!(
            "Cannot index {} with {}",
            base.type_name(),
            other.type_name()
        ))),
    }
}
