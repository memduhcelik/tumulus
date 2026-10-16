#[derive(Debug)]
pub enum Value {
    Null,
    Bool(bool),
    /// Text jq would print for this number.
    Number(String),
    String(String),
    Array(Vec<Value>),
    Object(Vec<(String, Value)>),
}

/// Past this depth the rest of a nest is freed on the heap. A wide array
/// still drops in a loop.
const DROP_RECURSION: usize = 32;

impl Drop for Value {
    fn drop(&mut self) {
        drop_in_place(self, 0);
    }
}

fn drop_in_place(value: &mut Value, depth: usize) {
    if depth >= DROP_RECURSION {
        drop_deep(value);
        return;
    }
    match value {
        Value::Array(items) => {
            for mut item in std::mem::take(items) {
                drop_in_place(&mut item, depth + 1);
            }
        }
        Value::Object(items) => {
            for (_, mut item) in std::mem::take(items) {
                drop_in_place(&mut item, depth + 1);
            }
        }
        _ => {}
    }
}

/// Children are removed before each value is dropped, so `Drop` sees an
/// empty container and does not walk the nest again.
fn drop_deep(value: &mut Value) {
    let mut stack = Vec::new();
    take_children(value, &mut stack);
    while let Some(mut next) = stack.pop() {
        take_children(&mut next, &mut stack);
    }
}

fn take_children(value: &mut Value, stack: &mut Vec<Value>) {
    match value {
        Value::Array(items) => stack.extend(std::mem::take(items)),
        Value::Object(items) => {
            stack.extend(std::mem::take(items).into_iter().map(|(_, item)| item));
        }
        _ => {}
    }
}

impl Clone for Value {
    fn clone(&self) -> Self {
        clone_at(self, 0)
    }
}

fn clone_at(value: &Value, depth: usize) -> Value {
    if depth >= DROP_RECURSION {
        return clone_deep(value);
    }
    match value {
        Value::Null => Value::Null,
        Value::Bool(bit) => Value::Bool(*bit),
        Value::Number(text) => Value::Number(text.clone()),
        Value::String(text) => Value::String(text.clone()),
        Value::Array(items) => {
            Value::Array(items.iter().map(|item| clone_at(item, depth + 1)).collect())
        }
        Value::Object(items) => Value::Object(
            items
                .iter()
                .map(|(key, item)| (key.clone(), clone_at(item, depth + 1)))
                .collect(),
        ),
    }
}

enum CloneJob<'a> {
    Expand(&'a Value),
    Array(usize),
    Object(Vec<String>),
}

/// Post-order, so a long chain does not use a stack frame per level.
fn clone_deep(value: &Value) -> Value {
    let mut jobs = vec![CloneJob::Expand(value)];
    let mut built = Vec::new();
    while let Some(job) = jobs.pop() {
        match job {
            CloneJob::Expand(Value::Array(items)) => {
                jobs.push(CloneJob::Array(items.len()));
                for item in items.iter().rev() {
                    jobs.push(CloneJob::Expand(item));
                }
            }
            CloneJob::Expand(Value::Object(items)) => {
                jobs.push(CloneJob::Object(
                    items.iter().map(|(key, _)| key.clone()).collect(),
                ));
                for (_, item) in items.iter().rev() {
                    jobs.push(CloneJob::Expand(item));
                }
            }
            CloneJob::Expand(Value::Null) => built.push(Value::Null),
            CloneJob::Expand(Value::Bool(bit)) => built.push(Value::Bool(*bit)),
            CloneJob::Expand(Value::Number(text)) => built.push(Value::Number(text.clone())),
            CloneJob::Expand(Value::String(text)) => built.push(Value::String(text.clone())),
            CloneJob::Array(count) => {
                let children = built.split_off(built.len() - count);
                built.push(Value::Array(children));
            }
            CloneJob::Object(keys) => {
                let children = built.split_off(built.len() - keys.len());
                built.push(Value::Object(keys.into_iter().zip(children).collect()));
            }
        }
    }
    built.pop().unwrap_or(Value::Null)
}

impl Value {
    pub(crate) fn into_string(mut self) -> Result<String, Self> {
        match &mut self {
            Value::String(text) => Ok(std::mem::take(text)),
            _ => Err(self),
        }
    }

    pub(crate) fn into_array(mut self) -> Result<Vec<Value>, Self> {
        match &mut self {
            Value::Array(items) => Ok(std::mem::take(items)),
            _ => Err(self),
        }
    }

    pub(crate) fn into_object(mut self) -> Result<Vec<(String, Value)>, Self> {
        match &mut self {
            Value::Object(items) => Ok(std::mem::take(items)),
            _ => Err(self),
        }
    }

    pub(crate) fn type_name(&self) -> &'static str {
        match self {
            Value::Null => "null",
            Value::Bool(_) => "boolean",
            Value::Number(_) => "number",
            Value::String(_) => "string",
            Value::Array(_) => "array",
            Value::Object(_) => "object",
        }
    }
}
