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
