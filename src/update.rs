//! Path updates. A path is collected from the input, then each write is applied
//! in that order. Array indexes keep their original slots until the update
//! finishes, so a later index is not shifted by an earlier deletion.

use crate::number::array_index;
use crate::print::{self, Layout, Style};
use crate::value::Value;

use super::{
    add_values, div_values, eval_in, fail, mul_values, rem_values, shown, slice_range, sub_values,
    truthy, Failure, Filter, UpdateOp,
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

fn combine_one(op: UpdateOp, paths: &[Vec<Step>], input: &Value, right: Value) -> Result<Value, String> {
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

fn locate(
    filter: &Filter,
    input: &Value,
    env: &[(String, Value)],
) -> Result<Vec<Found>, Failure> {
    match filter {
        Filter::Identity => Ok(vec![Found::Path(Vec::new())]),
        Filter::Field(key) => locate_key(input, key),
        Filter::Index(index) => locate_index(input, index),
        Filter::Slice { start, end } => locate_slice(input, start.clone(), end.clone()),
        Filter::Iterate => locate_iter(input),
        Filter::Recurse => Ok(locate_recurse(input, &[])),
        Filter::Optional(inner) => match locate(inner, input, env) {
            Ok(found) => Ok(found),
            Err(_) => Ok(Vec::new()),
        },
        Filter::Pipe(left, right) => locate_pipe(left, right, input, env),
        Filter::Comma(left, right) => {
            let mut found = locate(left, input, env)?;
            found.extend(locate(right, input, env)?);
            Ok(found)
        }
        Filter::Select(cond) => locate_select(cond, input, env),
        Filter::At { base, key, optional } => locate_at(base, key, *optional, input, env),
        other => {
            let values = eval_in(other, input, env)?;
            Ok(values.into_iter().map(Found::Broken).collect())
        }
    }
}

fn locate_key(input: &Value, key: &str) -> Result<Vec<Found>, Failure> {
    match input {
        Value::Null | Value::Object(_) => Ok(vec![Found::Path(vec![Step::Key(key.to_string())])]),
        other => Err(fail(format!(
            "Cannot index {} with string \"{key}\"",
            other.type_name()
        ))),
    }
}

fn locate_index(input: &Value, index: &super::ArrayIndex) -> Result<Vec<Found>, Failure> {
    let super::ArrayIndex::At(index) = index else {
        // A magnitude that does not fit is not a slot we can name.
        return Ok(vec![Found::Broken(Value::Null)]);
    };
    match input {
        Value::Null | Value::Array(_) => Ok(vec![Found::Path(vec![Step::Index(*index)])]),
        other => Err(fail(format!(
            "Cannot index {} with number",
            other.type_name()
        ))),
    }
}

fn locate_slice(
    input: &Value,
    start: Option<String>,
    end: Option<String>,
) -> Result<Vec<Found>, Failure> {
    match input {
        Value::Null | Value::Array(_) | Value::String(_) => {
            Ok(vec![Found::Path(vec![Step::Slice { start, end }])])
        }
        other => Err(fail(format!(
            "Cannot index {} with object",
            other.type_name()
        ))),
    }
}

fn locate_iter(input: &Value) -> Result<Vec<Found>, Failure> {
    match input {
        Value::Array(items) => Ok((0..items.len())
            .map(|index| Found::Path(vec![Step::Index(index as i64)]))
            .collect()),
        Value::Object(items) => Ok(items
            .iter()
            .map(|(key, _)| Found::Path(vec![Step::Key(key.clone())]))
            .collect()),
        other => Err(fail(format!(
            "Cannot iterate over {} ({})",
            other.type_name(),
            shown(other)
        ))),
    }
}

fn locate_recurse(value: &Value, prefix: &[Step]) -> Vec<Found> {
    let mut out = vec![Found::Path(prefix.to_vec())];
    match value {
        Value::Array(items) => {
            for (index, item) in items.iter().enumerate() {
                let mut path = prefix.to_vec();
                path.push(Step::Index(index as i64));
                out.extend(locate_recurse(item, &path));
            }
        }
        Value::Object(items) => {
            for (key, item) in items {
                let mut path = prefix.to_vec();
                path.push(Step::Key(key.clone()));
                out.extend(locate_recurse(item, &path));
            }
        }
        _ => {}
    }
    out
}

fn locate_pipe(
    left: &Filter,
    right: &Filter,
    input: &Value,
    env: &[(String, Value)],
) -> Result<Vec<Found>, Failure> {
    let mut out = Vec::new();
    for hit in locate(left, input, env)? {
        match hit {
            Found::Broken(value) => match eval_in(right, &value, env) {
                Ok(values) => out.extend(values.into_iter().map(Found::Broken)),
                Err(failure) => {
                    return Err(Failure {
                        partial: Vec::new(),
                        message: failure.message,
                    })
                }
            },
            Found::Path(prefix) => {
                let focused = value_at(input, &prefix).map_err(fail)?;
                for hit in locate(right, &focused, env)? {
                    match hit {
                        Found::Path(mut rest) => {
                            let mut full = prefix.clone();
                            full.append(&mut rest);
                            out.push(Found::Path(full));
                        }
                        broken => out.push(broken),
                    }
                }
            }
        }
    }
    Ok(out)
}

fn locate_select(
    cond: &Filter,
    input: &Value,
    env: &[(String, Value)],
) -> Result<Vec<Found>, Failure> {
    let flags = eval_in(cond, input, env)?;
    let mut out = Vec::new();
    for flag in flags {
        if truthy(&flag) {
            out.push(Found::Path(Vec::new()));
        }
    }
    Ok(out)
}

fn locate_at(
    base: &Filter,
    key: &Filter,
    optional: bool,
    input: &Value,
    env: &[(String, Value)],
) -> Result<Vec<Found>, Failure> {
    let bases = locate(base, input, env)?;
    let keys = match eval_in(key, input, env) {
        Ok(keys) => keys,
        Err(failure) if optional => failure.partial,
        Err(failure) => {
            return Err(Failure {
                partial: Vec::new(),
                message: failure.message,
            })
        }
    };
    let mut out = Vec::new();
    for key in &keys {
        for base in &bases {
            match base {
                Found::Broken(value) => match index_one(value, key) {
                    Ok(values) => out.extend(values.into_iter().map(Found::Broken)),
                    Err(_) if optional => {}
                    Err(message) => return Err(fail(message)),
                },
                Found::Path(steps) => {
                    let focused = value_at(input, steps).map_err(fail)?;
                    match step_for(key, &focused) {
                        Ok(step) => {
                            let mut path = steps.clone();
                            path.push(step);
                            out.push(Found::Path(path));
                        }
                        Err(_) if optional => {}
                        Err(message) => return Err(fail(message)),
                    }
                }
            }
        }
    }
    Ok(out)
}

fn index_one(value: &Value, key: &Value) -> Result<Vec<Value>, String> {
    match super::index_by(value, key) {
        Ok(values) => Ok(values),
        Err(failure) => Err(failure.message),
    }
}

fn step_for(key: &Value, container: &Value) -> Result<Step, String> {
    match key {
        Value::String(name) => match container {
            Value::Null | Value::Object(_) => Ok(Step::Key(name.clone())),
            other => Err(format!(
                "Cannot index {} with string \"{name}\"",
                other.type_name()
            )),
        },
        Value::Number(text) => {
            let Some(index) = array_index(text) else {
                return Err(format!(
                    "Cannot index {} with number",
                    container.type_name()
                ));
            };
            match container {
                Value::Null | Value::Array(_) => Ok(Step::Index(index)),
                other => Err(format!(
                    "Cannot index {} with number",
                    other.type_name()
                )),
            }
        }
        other => Err(format!(
            "Cannot index {} with {}",
            container.type_name(),
            other.type_name()
        )),
    }
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
            return Err(format!(
                "Cannot index {} with number",
                other.type_name()
            ));
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
        let current = read_edit(edit, &[Step::Slice {
            start: start.clone(),
            end: end.clone(),
        }])?;
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
            return Err(format!(
                "Cannot index {} with number",
                other.type_name()
            ));
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
        Edit::Obj(fields) => {
            Value::Object(fields.iter().map(|(key, value)| (key.clone(), freeze(value))).collect())
        }
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
