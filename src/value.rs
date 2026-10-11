#[derive(Debug)]
pub enum Value {
    Null,
    Bool(bool),
    /// Digits as they appeared in the input.
    Number(String),
    String(String),
    Array(Vec<Value>),
    Object(Vec<(String, Value)>),
}
