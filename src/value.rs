use std::rc::Rc;

/// A value several parents can hold. Cloning bumps the count. A write copies
/// the container first when another parent still has it, so the other parent
/// keeps the old contents.
#[derive(Debug)]
pub struct Share<T>(Rc<T>);

impl<T> Clone for Share<T> {
    fn clone(&self) -> Self {
        Share(Rc::clone(&self.0))
    }
}

impl<T> std::ops::Deref for Share<T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.0
    }
}

impl<T> Share<T> {
    pub fn new(value: T) -> Self {
        Share(Rc::new(value))
    }

    /// The value, with no other parent. `None` leaves this share in place.
    pub(crate) fn unique_mut(&mut self) -> Option<&mut T> {
        Rc::get_mut(&mut self.0)
    }
}

impl<T: Clone> Share<T> {
    /// A container this owner can write. Other owners keep the previous one.
    pub(crate) fn make_mut(&mut self) -> &mut T {
        Rc::make_mut(&mut self.0)
    }

    pub(crate) fn unwrap_or_clone(self) -> T {
        match Rc::try_unwrap(self.0) {
            Ok(value) => value,
            Err(shared) => (*shared).clone(),
        }
    }
}

impl<'a, T> IntoIterator for &'a Share<Vec<T>> {
    type Item = &'a T;
    type IntoIter = std::slice::Iter<'a, T>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}

#[derive(Debug)]
pub enum Value {
    Null,
    Bool(bool),
    /// Text jq would print for this number.
    Number(String),
    String(String),
    Array(Share<Vec<Value>>),
    Object(Share<Vec<(String, Value)>>),
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
            if let Some(items) = items.unique_mut() {
                for mut item in std::mem::take(items) {
                    drop_in_place(&mut item, depth + 1);
                }
            }
        }
        Value::Object(items) => {
            if let Some(items) = items.unique_mut() {
                for (_, mut item) in std::mem::take(items) {
                    drop_in_place(&mut item, depth + 1);
                }
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
        Value::Array(items) => {
            if let Some(items) = items.unique_mut() {
                stack.extend(std::mem::take(items));
            }
        }
        Value::Object(items) => {
            if let Some(items) = items.unique_mut() {
                stack.extend(std::mem::take(items).into_iter().map(|(_, item)| item));
            }
        }
        _ => {}
    }
}

impl Clone for Value {
    fn clone(&self) -> Self {
        match self {
            Value::Null => Value::Null,
            Value::Bool(bit) => Value::Bool(*bit),
            Value::Number(text) => Value::Number(text.clone()),
            Value::String(text) => Value::String(text.clone()),
            // The copy sees the same array or object until a write.
            Value::Array(items) => Value::Array(items.clone()),
            Value::Object(items) => Value::Object(items.clone()),
        }
    }
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
            Value::Array(items) => {
                let taken = std::mem::replace(items, Share::new(Vec::new()));
                Ok(taken.unwrap_or_clone())
            }
            _ => Err(self),
        }
    }

    pub(crate) fn into_object(mut self) -> Result<Vec<(String, Value)>, Self> {
        match &mut self {
            Value::Object(items) => {
                let taken = std::mem::replace(items, Share::new(Vec::new()));
                Ok(taken.unwrap_or_clone())
            }
            _ => Err(self),
        }
    }

    pub fn array(items: Vec<Value>) -> Self {
        Value::Array(Share::new(items))
    }

    pub fn object(pairs: Vec<(String, Value)>) -> Self {
        Value::Object(Share::new(pairs))
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
