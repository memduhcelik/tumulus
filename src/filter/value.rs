//! Arithmetic, comparison, and merging of values.

use crate::number;
use crate::value::Value;

use super::ast::*;
use super::eval::shown;

pub(super) fn apply_op(op: Op, left: &Value, right: &Value) -> Result<Value, String> {
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

pub(super) fn add_values(left: &Value, right: &Value) -> Result<Value, String> {
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

pub(super) fn sub_values(left: &Value, right: &Value) -> Result<Value, String> {
    match (left, right) {
        (Value::Number(a), Value::Number(b)) => Ok(num_binop(a, b, NumOp::Sub)),
        (Value::Array(a), Value::Array(b)) => Ok(array_sub(a, b)),
        _ => Err(binary_type(left, right, "cannot be subtracted")),
    }
}

pub(super) fn mul_values(left: &Value, right: &Value) -> Result<Value, String> {
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

pub(super) fn div_values(left: &Value, right: &Value) -> Result<Value, String> {
    match (left, right) {
        (Value::Number(a), Value::Number(b)) => num_div(left, right, a, b),
        (Value::String(text), Value::String(sep)) => Ok(split_string(text, sep)),
        _ => Err(binary_type(left, right, "cannot be divided")),
    }
}

pub(super) fn rem_values(left: &Value, right: &Value) -> Result<Value, String> {
    match (left, right) {
        (Value::Number(a), Value::Number(b)) => num_rem(left, right, a, b),
        _ => Err(binary_type(left, right, "cannot be divided (remainder)")),
    }
}

pub(super) enum NumOp {
    Add,
    Sub,
    Mul,
}

pub(super) fn num_binop(left: &str, right: &str, op: NumOp) -> Value {
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

pub(super) fn num_div(left: &Value, right: &Value, a: &str, b: &str) -> Result<Value, String> {
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

pub(super) fn num_rem(left: &Value, right: &Value, a: &str, b: &str) -> Result<Value, String> {
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

pub(super) fn negate_value(value: &Value) -> Result<Value, String> {
    let Value::Number(text) = value else {
        return Err(format!(
            "{} ({}) cannot be negated",
            value.type_name(),
            shown(value)
        ));
    };
    // The sign bit flips, including NaN and zero. `-(nan)` is a negative NaN.
    let negated = -number::f64_of(text);
    Ok(Value::Number(number::mark_f64(number::format_jq_f64(
        negated,
    ))))
}

/// `None` means the repeat would be enormous. jq builds it; we refuse.
pub(super) fn string_times(text: &str, n: number::Real) -> Option<Value> {
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

pub(super) fn split_string(text: &str, sep: &str) -> Value {
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

pub(super) fn array_sub(left: &[Value], right: &[Value]) -> Value {
    let mut out = Vec::new();
    for item in left {
        if !right.iter().any(|other| cmp_value(item, other) == 0) {
            out.push(item.clone());
        }
    }
    Value::Array(out)
}

pub(super) fn merge_objects(left: &[(String, Value)], right: &[(String, Value)]) -> Value {
    let mut pairs = left.to_vec();
    for (key, value) in right {
        put(&mut pairs, key.clone(), value.clone());
    }
    Value::Object(pairs)
}

pub(super) fn merge_deep(left: &[(String, Value)], right: &[(String, Value)]) -> Value {
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

pub(super) fn type_rank(value: &Value) -> u8 {
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

enum Cmp<'a> {
    Pair(&'a Value, &'a Value),
    Array {
        left: &'a [Value],
        right: &'a [Value],
        index: usize,
    },
    Object {
        left: &'a [(String, Value)],
        right: &'a [(String, Value)],
        keys: Vec<&'a str>,
        index: usize,
    },
}

/// Element order is the same as a recursive walk. The stack is on the heap
/// so a nest of tens of thousands of arrays does not overflow.
pub(super) fn cmp_value(left: &Value, right: &Value) -> i8 {
    let mut stack = vec![Cmp::Pair(left, right)];
    while let Some(job) = stack.pop() {
        match job {
            Cmp::Pair(left, right) => {
                let left_rank = type_rank(left);
                let right_rank = type_rank(right);
                if left_rank != right_rank {
                    return if left_rank < right_rank { -1 } else { 1 };
                }
                match (left, right) {
                    (Value::Number(a), Value::Number(b)) => {
                        let cmp = cmp_number(a, b);
                        if cmp != 0 {
                            return cmp;
                        }
                    }
                    (Value::String(a), Value::String(b)) => {
                        let cmp = ord_i8(a.cmp(b));
                        if cmp != 0 {
                            return cmp;
                        }
                    }
                    (Value::Array(a), Value::Array(b)) => {
                        stack.push(Cmp::Array {
                            left: a,
                            right: b,
                            index: 0,
                        });
                    }
                    (Value::Object(a), Value::Object(b)) => match object_keys(a, b) {
                        Err(cmp) => return cmp,
                        Ok(keys) => stack.push(Cmp::Object {
                            left: a,
                            right: b,
                            keys,
                            index: 0,
                        }),
                    },
                    _ => {}
                }
            }
            Cmp::Array { left, right, index } => {
                if index >= left.len() || index >= right.len() {
                    let cmp = ord_i8(left.len().cmp(&right.len()));
                    if cmp != 0 {
                        return cmp;
                    }
                    continue;
                }
                stack.push(Cmp::Array {
                    left,
                    right,
                    index: index + 1,
                });
                stack.push(Cmp::Pair(&left[index], &right[index]));
            }
            Cmp::Object {
                left,
                right,
                keys,
                index,
            } => {
                if index >= keys.len() {
                    continue;
                }
                let left_value = object_get(left, keys[index]);
                let right_value = object_get(right, keys[index]);
                stack.push(Cmp::Object {
                    left,
                    right,
                    keys,
                    index: index + 1,
                });
                stack.push(Cmp::Pair(left_value, right_value));
            }
        }
    }
    0
}

/// Sorted keys when the two objects have the same names. Otherwise the
/// comparison of the names.
fn object_keys<'a>(
    left: &'a [(String, Value)],
    right: &'a [(String, Value)],
) -> Result<Vec<&'a str>, i8> {
    let mut left_keys: Vec<&str> = left.iter().map(|(key, _)| key.as_str()).collect();
    let mut right_keys: Vec<&str> = right.iter().map(|(key, _)| key.as_str()).collect();
    left_keys.sort_unstable();
    right_keys.sort_unstable();
    for (a, b) in left_keys.iter().zip(right_keys.iter()) {
        match a.cmp(b) {
            std::cmp::Ordering::Equal => {}
            std::cmp::Ordering::Less => return Err(-1),
            std::cmp::Ordering::Greater => return Err(1),
        }
    }
    if left_keys.len() != right_keys.len() {
        return Err(if left_keys.len() < right_keys.len() {
            -1
        } else {
            1
        });
    }
    Ok(left_keys)
}

pub(super) fn cmp_number(left: &str, right: &str) -> i8 {
    number::cmp_number_text(left, right)
}

pub(super) fn object_get<'a>(items: &'a [(String, Value)], key: &str) -> &'a Value {
    items
        .iter()
        .find(|(name, _)| name == key)
        .map(|(_, value)| value)
        .expect("key is present")
}

pub(super) fn ord_i8(order: std::cmp::Ordering) -> i8 {
    match order {
        std::cmp::Ordering::Less => -1,
        std::cmp::Ordering::Equal => 0,
        std::cmp::Ordering::Greater => 1,
    }
}

pub(super) fn binary_type(left: &Value, right: &Value, verb: &str) -> String {
    format!(
        "{} ({}) and {} ({}) {verb}",
        left.type_name(),
        shown(left),
        right.type_name(),
        shown(right)
    )
}

pub(super) fn zero_divisor(left: &Value, right: &Value, remainder: bool) -> String {
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

pub(super) fn put(pairs: &mut Vec<(String, Value)>, key: String, value: Value) {
    if let Some((_, slot)) = pairs.iter_mut().find(|(name, _)| name == &key) {
        *slot = value;
    } else {
        pairs.push((key, value));
    }
}
