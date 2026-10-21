//! `sort`, `sort_by`, `group_by`, `unique`, `unique_by`, `min`, `max`,
//! `min_by`, and `max_by`.
//!
//! Keys are every value the filter yields, in order. A tie in `min` keeps
//! the earlier value. A tie in `max` keeps the later one. `NaN` compares
//! less than itself, so `min` walks to the last `NaN` and `max` keeps the
//! first. Grouping uses equality, and `NaN` is not equal to `NaN`.

use std::cmp::Ordering;

use crate::value::Value;

use super::ast::*;
use super::eval::{self, bare_fail, fail, iterate, shown};
use super::value::cmp_value;

pub(super) fn eval_order(
    kind: SortKind,
    by: Option<&Filter>,
    input: &Value,
    env: &[(String, Value)],
) -> Result<Vec<Value>, Failure> {
    if !matches!(input, Value::Array(_) | Value::Object(_)) {
        return Err(if kind.pairs_input() {
            both_iterated(input, input)
        } else if matches!(kind, SortKind::Sort) {
            not_array(input)
        } else {
            fail(iterate_message(input))
        });
    }
    if matches!(input, Value::Object(_)) {
        return Err(object_order(kind, by, input, env)?);
    }
    let Value::Array(items) = input else {
        return Err(not_array(input));
    };
    if matches!(kind, SortKind::Sort) {
        return Ok(vec![Value::array(sorted(items))]);
    }
    let keyed = keys_of(items, by, env)?;
    let value = match kind {
        SortKind::SortBy => Value::array(arrange(keyed, false)),
        SortKind::Group | SortKind::Unique | SortKind::UniqueBy => {
            let groups = arrange(keyed, true);
            if matches!(kind, SortKind::Group) {
                Value::array(groups)
            } else {
                Value::array(firsts(groups))
            }
        }
        SortKind::Min | SortKind::MinBy => extreme(&keyed, false),
        SortKind::Max | SortKind::MaxBy => extreme(&keyed, true),
        SortKind::Sort => unreachable!("sort has no key"),
    };
    Ok(vec![value])
}

/// An object can be walked, but it is not an array, so ordering rejects it
/// after the key filter has run. `min` and `max` do not run a key.
fn object_order(
    kind: SortKind,
    by: Option<&Filter>,
    input: &Value,
    env: &[(String, Value)],
) -> Result<Failure, Failure> {
    if kind.pairs_input() {
        return Ok(both_iterated(input, input));
    }
    if matches!(kind, SortKind::Sort) {
        return Ok(not_array(input));
    }
    let Value::Object(fields) = input else {
        return Ok(not_array(input));
    };
    let mut rows = Vec::with_capacity(fields.len());
    for (_, value) in fields {
        // `unique` has no filter. Each value is its own one-element key,
        // which is what `map([.])` produces.
        let key = match by {
            Some(filter) => key_of(filter, value, env)?,
            None => Value::array(vec![value.clone()]),
        };
        rows.push(key);
    }
    let keys = Value::array(rows);
    Ok(if matches!(kind, SortKind::MinBy | SortKind::MaxBy) {
        both_iterated(input, &keys)
    } else {
        fail(format!(
            "{} ({}) and {} ({}) cannot be sorted, as they are not both arrays",
            input.type_name(),
            shown(input),
            keys.type_name(),
            shown(&keys)
        ))
    })
}

fn keys_of(
    items: &[Value],
    by: Option<&Filter>,
    env: &[(String, Value)],
) -> Result<Vec<(Value, Value)>, Failure> {
    let mut keyed = Vec::with_capacity(items.len());
    for item in items {
        let key = match by {
            Some(filter) => key_of(filter, item, env)?,
            None => Value::array(vec![item.clone()]),
        };
        keyed.push((key, item.clone()));
    }
    Ok(keyed)
}

fn key_of(filter: &Filter, item: &Value, env: &[(String, Value)]) -> Result<Value, Failure> {
    match eval::eval_in(filter, item, env) {
        Ok(values) => Ok(Value::array(values)),
        Err(failure) => Err(bare_fail(failure)),
    }
}

fn sorted(items: &[Value]) -> Vec<Value> {
    let mut order: Vec<usize> = (0..items.len()).collect();
    order.sort_by(|&left, &right| sort_ord(&items[left], &items[right]));
    order
        .into_iter()
        .map(|index| items[index].clone())
        .collect()
}

/// `grouped` keeps one array per run of equal keys. Otherwise the values
/// come back in key order.
fn arrange(keyed: Vec<(Value, Value)>, grouped: bool) -> Vec<Value> {
    let mut order: Vec<usize> = (0..keyed.len()).collect();
    order.sort_by(|&left, &right| sort_ord(&keyed[left].0, &keyed[right].0));
    if !grouped {
        return order
            .into_iter()
            .map(|index| keyed[index].1.clone())
            .collect();
    }
    let mut groups: Vec<Value> = Vec::new();
    let mut previous: Option<Value> = None;
    for index in order {
        let (key, value) = &keyed[index];
        let same = previous
            .as_ref()
            .is_some_and(|before| cmp_value(before, key) == 0);
        if same {
            if let Some(Value::Array(group)) = groups.last_mut() {
                group.make_mut().push(value.clone());
            }
        } else {
            groups.push(Value::array(vec![value.clone()]));
            previous = Some(key.clone());
        }
    }
    groups
}

fn firsts(groups: Vec<Value>) -> Vec<Value> {
    groups
        .into_iter()
        .filter_map(|group| match group.into_array() {
            Ok(mut items) => items.drain(..).next(),
            Err(other) => Some(other),
        })
        .collect()
}

/// `min` replaces the choice only when the new key is strictly smaller.
/// `max` also replaces it when the keys are equal, so the later value wins.
/// `NaN` is smaller than itself, so `min` ends on the last `NaN`.
fn extreme(keyed: &[(Value, Value)], max: bool) -> Value {
    if keyed.is_empty() {
        return Value::Null;
    }
    let mut best = 0usize;
    for index in 1..keyed.len() {
        let cmp = cmp_value(&keyed[index].0, &keyed[best].0);
        let replace = if max { cmp >= 0 } else { cmp < 0 };
        if replace {
            best = index;
        }
    }
    keyed[best].1.clone()
}

/// `NaN` is less than `NaN`, which is not an ordering a sort can use.
/// Those two keys stay in the order they arrived.
fn sort_ord(left: &Value, right: &Value) -> Ordering {
    let forward = cmp_value(left, right);
    if forward < 0 && cmp_value(right, left) < 0 {
        Ordering::Equal
    } else if forward < 0 {
        Ordering::Less
    } else if forward > 0 {
        Ordering::Greater
    } else {
        Ordering::Equal
    }
}

fn not_array(value: &Value) -> Failure {
    fail(format!(
        "{} ({}) cannot be sorted, as it is not an array",
        value.type_name(),
        shown(value)
    ))
}

fn both_iterated(left: &Value, right: &Value) -> Failure {
    fail(format!(
        "{} ({}) and {} ({}) cannot be iterated over",
        left.type_name(),
        shown(left),
        right.type_name(),
        shown(right)
    ))
}

fn iterate_message(value: &Value) -> String {
    match iterate(value) {
        Err(failure) => failure.message,
        Ok(_) => format!(
            "Cannot iterate over {} ({})",
            value.type_name(),
            shown(value)
        ),
    }
}
