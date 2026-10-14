//! Filter expressions, patterns, and failures.

use crate::math;
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
    /// A fixed bracket index, as in `.[1, 2]`. `text` is the spelling `path` prints.
    Index {
        at: ArrayIndex,
        text: String,
    },
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
    /// `optional` is set when `?` is written on this bracket, so a failed
    /// index is skipped and the next key still runs. `?` after parentheses
    /// leaves the flag off.
    At {
        base: Box<Filter>,
        key: Box<Filter>,
        optional: bool,
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
    /// `$name`
    Var(String),
    /// `expr as pattern | body`. `.` in the body is the input of this expression.
    Bind {
        expr: Box<Filter>,
        pattern: Pattern,
        body: Box<Filter>,
    },
    Reduce {
        stream: Box<Filter>,
        pattern: Pattern,
        init: Box<Filter>,
        update: Box<Filter>,
    },
    Foreach {
        stream: Box<Filter>,
        pattern: Pattern,
        init: Box<Filter>,
        update: Box<Filter>,
        /// `None` emits each update. `Some` emits the extract instead.
        extract: Option<Box<Filter>>,
    },
    /// `empty`. No outputs.
    Empty,
    /// `length`
    Length,
    /// `keys`
    Keys,
    /// `has(expr)`
    Has(Box<Filter>),
    /// `select(expr)`
    Select(Box<Filter>),
    /// `map(expr)`. One array, from every output on each element or value.
    Map(Box<Filter>),
    /// `type`
    Type,
    /// `tonumber`
    ToNumber,
    /// `tostring`
    ToString,
    /// `join(expr)`
    Join(Box<Filter>),
    /// `ltrimstr(expr)`
    LTrim(Box<Filter>),
    /// `rtrimstr(expr)`
    RTrim(Box<Filter>),
    /// `todate`
    ToDate,
    /// `fromdate`
    FromDate,
    /// `gmtime`
    GmTime,
    /// `localtime`
    LocalTime,
    /// `mktime`
    MkTime,
    /// `strftime(fmt)`
    StrfTime(Box<Filter>),
    /// `strflocaltime(fmt)`
    StrfLocalTime(Box<Filter>),
    /// `strptime(fmt)`
    StrpTime(Box<Filter>),
    /// `now`
    Now,
    /// `floor`, `sqrt`, and the other one-number libm functions.
    Math1(math::Unary),
    /// `pow`, `atan2`, `ldexp`, and the other two-argument libm functions.
    /// The second filter runs outermost and is not run again for each value
    /// of the first. The first runs again for every value of the second.
    Math2(math::Binary, Box<Filter>, Box<Filter>),
    /// `fma(x; y; z)`. The third filter runs outermost.
    Math3(Box<Filter>, Box<Filter>, Box<Filter>),
    /// `if . < 0 then -. else . end`.
    Abs,
    /// Raises `Error: pow10/0 not found at build time`.
    Pow10,
    /// `error` with no argument. Raises the current input.
    Error,
    /// `error(expr)`. A call written after it is a syntax error.
    Raise(Box<Filter>),
    /// `try body` or `try body catch handler`. No handler means `catch empty`.
    Try {
        body: Box<Filter>,
        handler: Option<Box<Filter>>,
    },
    /// `=`, `|=`, or one of `+=` `-=` `*=` `/=` `%=` `//=`.
    Update {
        op: UpdateOp,
        path: Box<Filter>,
        expr: Box<Filter>,
    },
    /// `path(expr)`.
    PathOf(Box<Filter>),
    /// `paths`, or `paths(expr)` when the filter is present.
    Paths(Option<Box<Filter>>),
    /// `getpath(expr)`.
    GetPath(Box<Filter>),
    /// `setpath(path; value)`. The value filter is the rightmost.
    SetPath(Box<Filter>, Box<Filter>),
    /// `delpaths(expr)`.
    DelPaths(Box<Filter>),
}

/// How an update writes. `Modify` is `|=`. `Alt` is `//=`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UpdateOp {
    Assign,
    Modify,
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Alt,
}

/// A name, or an array or object taken apart by indexing.
#[derive(Clone, Debug)]
pub enum Pattern {
    Var(String),
    Array(Vec<Pattern>),
    Object(Vec<PatField>),
}

/// One field of an object pattern. `{$name: pat}` sets `also` to that name.
#[derive(Clone, Debug)]
pub struct PatField {
    pub key: Box<Filter>,
    pub pattern: Pattern,
    /// Bound to the whole indexed value, before `pattern`.
    pub also: Option<String>,
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

#[derive(Clone)]
pub struct Failure {
    pub partial: Vec<Value>,
    pub message: String,
    /// Set when `error` raised this value. Builtin failures leave it empty,
    /// and `catch` receives the message text.
    pub caught: Option<Value>,
}
