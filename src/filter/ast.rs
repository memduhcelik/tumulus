//! Filter expressions, patterns, and failures.

use std::cell::RefCell;
use std::rc::{Rc, Weak};

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
    /// `.[start:end]`, or `base[start:end]`. `None` means that side was left off.
    /// Bounds run on the value the brackets were applied to, and `base` runs
    /// again for every pair of bounds. A missing `base` is that value itself.
    Slice {
        base: Option<Box<Filter>>,
        start: Option<Box<Filter>>,
        end: Option<Box<Filter>>,
    },
    Iterate,
    /// `..` and `recurse` with no arguments. The value, then every value inside it.
    Recurse,
    /// `recurse(f)` or `recurse(f; cond)`. Depth first. `cond` is checked on
    /// each output of `f`, and that output is entered once per truthy result.
    RecurseDown {
        step: Box<Filter>,
        cond: Option<Box<Filter>>,
    },
    /// `walk(f)`. Arrays and objects are rebuilt from the bottom, then `f` runs.
    Walk(Box<Filter>),
    /// `to_entries`.
    ToEntries,
    /// `from_entries`.
    FromEntries,
    /// `with_entries(f)`.
    WithEntries(Box<Filter>),
    /// `tojson`. Compact JSON, with a string quoted.
    ToJson,
    /// `add`. `reduce .[] as $x (null; . + $x)`.
    Add,
    Pipe(Box<Filter>, Box<Filter>),
    Comma(Box<Filter>, Box<Filter>),
    Optional(Box<Filter>),
    Literal(Value),
    /// Pieces of a string, with `\( )` between them.
    Interpolate(Vec<Piece>),
    /// `@name`. The name is checked when a value is formatted.
    Format(String),
    /// `@name "text \(expr)"`. Literal text stays as written. Each
    /// interpolation is formatted on its own. With no interpolation the
    /// result is that text, and the name is not checked.
    FormatString {
        name: String,
        pieces: Vec<Piece>,
    },
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
    /// `$__loc__`. The line is the line of that `$` in the program.
    /// The file is always `<top-level>`, including a program read with `-f`.
    Here(usize),
    /// `input`. The next value from the program's input, or `break` at the end.
    Input,
    /// `inputs`. Every remaining value. The final `break` is not a result.
    Inputs,
    /// `input_filename`. The file of the value most recently read, or null.
    InputFilename,
    /// `env`. The process environment, not the binding of `$ENV`.
    Environ,
    /// `sort`, `unique`, `min`, `max`, and the `*_by` forms.
    Order {
        kind: SortKind,
        /// The key filter. Absent for `sort`, `unique`, `min`, and `max`.
        by: Option<Box<Filter>>,
    },
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
    /// `ascii_downcase`. Only ASCII `A`–`Z` change.
    AsciiDown,
    /// `ascii_upcase`. Only ASCII `a`–`z` change.
    AsciiUp,
    /// `explode`. A string becomes its code points.
    Explode,
    /// `implode`. Code points become a string.
    Implode,
    /// `utf8bytelength`
    Utf8ByteLength,
    /// `startswith(expr)`
    StartsWith(Box<Filter>),
    /// `endswith(expr)`
    EndsWith(Box<Filter>),
    /// `index(expr)`
    IndexOf(Box<Filter>),
    /// `rindex(expr)`
    RIndexOf(Box<Filter>),
    /// `indices(expr)`
    IndicesOf(Box<Filter>),
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
    /// `limit(count; expr)`. `count` runs outermost. A later value of `expr`
    /// is not run once `count` values have been kept.
    Limit(Box<Filter>, Box<Filter>),
    /// `first`, or `first(expr)`. No filter means `.[0]`.
    First(Option<Box<Filter>>),
    /// `last`, or `last(expr)`. No filter means `.[-1]`.
    /// `last(expr)` is the last value, or `null` when `expr` yields nothing.
    Last(Option<Box<Filter>>),
    /// `nth(index)`, or `nth(index; expr)`. No `expr` means `.[index]`.
    Nth {
        index: Box<Filter>,
        expr: Option<Box<Filter>>,
    },
    /// `range(to)`, `range(from; to)`, or `range(from; to; step)`.
    /// One argument starts at 0. One or two arguments step by 1, and keep
    /// going when the end is `nan`.
    Range {
        from: Option<Box<Filter>>,
        to: Box<Filter>,
        step: Option<Box<Filter>>,
    },
    /// `until(cond; next)`.
    Until(Box<Filter>, Box<Filter>),
    /// `while(cond; update)`.
    While(Box<Filter>, Box<Filter>),
    /// `repeat(expr)`. `expr` is run on the original input each time.
    Repeat(Box<Filter>),
    /// `def name: body; rest`. `func` owns the body. A call holds only a weak
    /// pointer, so a function that calls itself does not keep a cycle.
    Define {
        func: Rc<FnCell>,
        rest: Box<Filter>,
    },
    /// A call of one arity of a function defined in this program.
    Call {
        func: Weak<FnCell>,
        args: Vec<Filter>,
    },
    /// A filter argument of the function that is running.
    Param(String),
    /// A name or an arity nobody defined. Reported when the call is reached.
    Unresolved {
        at: usize,
        name: String,
        arity: usize,
        args: Vec<Filter>,
    },
}

/// A parameter of a user function. `$name` is bound before the body runs.
/// A bare name is a filter, re-run at each use.
#[derive(Clone, Debug)]
pub enum FnParam {
    Filter(String),
    Value(String),
}

/// One user function. The body is filled in after the name is in scope, so
/// the function can call itself. `Rc` keeps that body shared with every call.
#[derive(Debug)]
pub struct FnCell {
    pub params: Vec<FnParam>,
    pub body: RefCell<Rc<Filter>>,
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

/// Which ordering builtin is running.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SortKind {
    Sort,
    SortBy,
    Group,
    Unique,
    UniqueBy,
    Min,
    Max,
    MinBy,
    MaxBy,
}

impl SortKind {
    /// `min` and `max` name both values in the "iterated" message.
    /// The `*_by` forms build a key array first.
    pub(crate) fn pairs_input(self) -> bool {
        matches!(self, SortKind::Min | SortKind::Max)
    }
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
