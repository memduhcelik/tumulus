//! Reading a program into a filter.

use crate::math::{self, Kind};
use crate::number::{self, array_index};
use crate::value::Value;

use super::ast::*;
use super::eval::shown;
use super::value::{apply_op, put};

pub fn compile(source: &str) -> Result<Filter, ProgramError> {
    compile_with(source, &[])
}

/// `scope` names variables that are already bound, ahead of the program.
pub fn compile_with(source: &str, scope: &[String]) -> Result<Filter, ProgramError> {
    let mut parser = Parser {
        src: source,
        index: 0,
        reject_block: false,
        reject_try: false,
        suffix_question: false,
    };
    parser.skip_ws();
    let filter = parser.parse_pipe()?;
    parser.skip_ws();
    if !parser.eof() {
        return Err(ProgramError::Syntax);
    }
    check_vars(&filter, scope)?;
    Ok(filter)
}

pub(super) fn check_vars(filter: &Filter, scope: &[String]) -> Result<(), ProgramError> {
    match filter {
        Filter::Identity
        | Filter::Field(_)
        | Filter::Index { .. }
        | Filter::Paths(None)
        | Filter::Slice { .. }
        | Filter::Iterate
        | Filter::Recurse
        | Filter::Literal(_)
        | Filter::Not
        | Filter::Empty
        | Filter::Length
        | Filter::Keys
        | Filter::Type
        | Filter::ToNumber
        | Filter::ToString
        | Filter::ToDate
        | Filter::FromDate
        | Filter::GmTime
        | Filter::LocalTime
        | Filter::MkTime
        | Filter::Now
        | Filter::Math1(_)
        | Filter::Abs
        | Filter::Pow10
        | Filter::Error => Ok(()),
        Filter::Var(name) => {
            if scope.iter().any(|bound| bound == name) {
                Ok(())
            } else {
                Err(ProgramError::Syntax)
            }
        }
        Filter::Pipe(left, right)
        | Filter::Comma(left, right)
        | Filter::And(left, right)
        | Filter::Or(left, right)
        | Filter::Alt(left, right) => {
            check_vars(left, scope)?;
            check_vars(right, scope)
        }
        Filter::Bin { left, right, .. } => {
            check_vars(left, scope)?;
            check_vars(right, scope)
        }
        Filter::Optional(inner)
        | Filter::Collect(inner)
        | Filter::Neg(inner)
        | Filter::Has(inner)
        | Filter::Select(inner)
        | Filter::Map(inner)
        | Filter::Join(inner)
        | Filter::LTrim(inner)
        | Filter::RTrim(inner)
        | Filter::StrfTime(inner)
        | Filter::StrfLocalTime(inner)
        | Filter::StrpTime(inner)
        | Filter::Raise(inner) => check_vars(inner, scope),
        Filter::Try { body, handler } => {
            check_vars(body, scope)?;
            if let Some(handler) = handler {
                check_vars(handler, scope)?;
            }
            Ok(())
        }
        Filter::Update { path, expr, .. } => {
            check_vars(path, scope)?;
            check_vars(expr, scope)
        }
        Filter::PathOf(inner) | Filter::GetPath(inner) | Filter::DelPaths(inner) => {
            check_vars(inner, scope)
        }
        Filter::Paths(Some(inner)) => check_vars(inner, scope),
        Filter::SetPath(path, value) => {
            check_vars(path, scope)?;
            check_vars(value, scope)
        }
        Filter::Interpolate(pieces) => {
            for piece in pieces {
                if let Piece::Interp(inner) = piece {
                    check_vars(inner, scope)?;
                }
            }
            Ok(())
        }
        Filter::Object(fields) => {
            for field in fields {
                match field {
                    ObjField::Pair { key, value } => {
                        check_vars(key, scope)?;
                        check_vars(value, scope)?;
                    }
                    ObjField::Lookup(key) => check_vars(key, scope)?,
                }
            }
            Ok(())
        }
        Filter::At { base, key, .. } => {
            check_vars(base, scope)?;
            check_vars(key, scope)
        }
        Filter::Math2(_, left, right) => {
            check_vars(left, scope)?;
            check_vars(right, scope)
        }
        Filter::Math3(x, y, z) => {
            check_vars(x, scope)?;
            check_vars(y, scope)?;
            check_vars(z, scope)
        }
        Filter::If { cond, yes, no } => {
            check_vars(cond, scope)?;
            check_vars(yes, scope)?;
            check_vars(no, scope)
        }
        Filter::Bind {
            expr,
            pattern,
            body,
        } => {
            check_vars(expr, scope)?;
            check_pattern(pattern, scope)?;
            let mut inner = scope.to_vec();
            pattern_names(pattern, &mut inner);
            check_vars(body, &inner)
        }
        Filter::Reduce {
            stream,
            pattern,
            init,
            update,
        } => {
            check_vars(stream, scope)?;
            check_vars(init, scope)?;
            check_pattern(pattern, scope)?;
            let mut inner = scope.to_vec();
            pattern_names(pattern, &mut inner);
            check_vars(update, &inner)
        }
        Filter::Foreach {
            stream,
            pattern,
            init,
            update,
            extract,
        } => {
            check_vars(stream, scope)?;
            check_vars(init, scope)?;
            check_pattern(pattern, scope)?;
            let mut inner = scope.to_vec();
            pattern_names(pattern, &mut inner);
            check_vars(update, &inner)?;
            if let Some(extract) = extract {
                check_vars(extract, &inner)?;
            }
            Ok(())
        }
    }
}

/// Keys see the surrounding scope only, not names this pattern binds.
pub(super) fn check_pattern(pattern: &Pattern, scope: &[String]) -> Result<(), ProgramError> {
    match pattern {
        Pattern::Var(_) => Ok(()),
        Pattern::Array(items) => {
            for item in items {
                check_pattern(item, scope)?;
            }
            Ok(())
        }
        Pattern::Object(fields) => {
            for field in fields {
                check_vars(&field.key, scope)?;
                check_pattern(&field.pattern, scope)?;
            }
            Ok(())
        }
    }
}

pub(super) fn pattern_names(pattern: &Pattern, out: &mut Vec<String>) {
    match pattern {
        Pattern::Var(name) => out.push(name.clone()),
        Pattern::Array(items) => {
            for item in items {
                pattern_names(item, out);
            }
        }
        Pattern::Object(fields) => {
            for field in fields {
                if let Some(name) = &field.also {
                    out.push(name.clone());
                }
                pattern_names(&field.pattern, out);
            }
        }
    }
}

pub(super) struct Parser<'a> {
    src: &'a str,
    index: usize,
    /// `if`, `reduce`, and `foreach` are not terms. A fold stream uses this
    /// so `reduce if` stays a syntax error while `(if ...)` is still a term.
    reject_block: bool,
    /// Object values reject `try`. Parentheses and arguments clear it.
    reject_try: bool,
    /// The leading minus of a `try` operand. `?` there marks only the last suffix.
    suffix_question: bool,
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

    /// `//` is right-associative. It is looser than an update and tighter than `,`.
    fn parse_alt(&mut self) -> Result<Filter, ProgramError> {
        let left = self.parse_update()?;
        if self.eat_alt() {
            let right = self.parse_alt()?;
            return Ok(Filter::Alt(Box::new(left), Box::new(right)));
        }
        Ok(left)
    }

    /// `=` does not chain. `//=` is one operator, not `//` followed by `=`.
    fn parse_update(&mut self) -> Result<Filter, ProgramError> {
        let left = self.parse_or()?;
        let Some(op) = self.take_update() else {
            return Ok(left);
        };
        let right = self.parse_or()?;
        if self.sees_update() {
            return Err(ProgramError::Syntax);
        }
        Ok(Filter::Update {
            op,
            path: Box::new(left),
            expr: Box::new(right),
        })
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
            // `+=` and `-=` are updates, not addition followed by assignment.
            let op = match (self.peek(), self.peek_at(1)) {
                (Some('+'), Some('=')) | (Some('-'), Some('=')) => break,
                (Some('+'), _) => Op::Add,
                (Some('-'), _) => Op::Sub,
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
        let mut left = self.parse_binding()?;
        while let Some(op) = self.mul_op() {
            // A minus here is the start of the operand, and that operand
            // includes the tighter `*` `/` `%` that follow it: `2/-3*4`
            // is `2/-(3*4)`. Without the minus, `8/2/2` stays left-associative.
            let right = if self.sees('-') {
                self.parse_signed()?
            } else {
                self.parse_binding()?
            };
            left = Filter::Bin {
                op,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    /// `as` binds tighter than `*` and a leading `-`. The body is the rest of
    /// the expression, and `.` there is still the input of the `as`.
    fn parse_binding(&mut self) -> Result<Filter, ProgramError> {
        let left = self.parse_juxt()?;
        if !self.eat_keyword("as") {
            return Ok(left);
        }
        let pattern = self.parse_pattern()?;
        if !self.eat('|') {
            return Err(ProgramError::Syntax);
        }
        let body = self.parse_pipe()?;
        Ok(Filter::Bind {
            expr: Box::new(left),
            pattern,
            body: Box::new(body),
        })
    }

    fn parse_juxt(&mut self) -> Result<Filter, ProgramError> {
        let mut filter = self.parse_postfix()?;
        loop {
            self.skip_ws();
            if matches!(self.peek(), Some('.' | '(')) {
                // `length(1)` and `map(f)(1)` are calls. A call this version
                // does not take is a syntax error, not a pipe.
                if self.peek() == Some('(') && forbids_call(&filter) {
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
                filter = if self.suffix_question {
                    optional_last(filter)
                } else {
                    Filter::Optional(Box::new(filter))
                };
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
            let saved_block = self.reject_block;
            let saved_try = self.reject_try;
            self.reject_block = false;
            self.reject_try = false;
            let inner = self.parse_pipe();
            self.reject_block = saved_block;
            self.reject_try = saved_try;
            let inner = inner?;
            self.skip_ws();
            if !self.eat(')') {
                return Err(ProgramError::Syntax);
            }
            return Ok(inner);
        }
        if self.eat('$') {
            let name = self.ident()?;
            return Ok(Filter::Var(name));
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
            let at = self.index;
            let name = self.ident()?;
            if let Some(filter) = self.parse_math(at, &name)? {
                return Ok(filter);
            }
            return match name.as_str() {
                "true" => Ok(Filter::Literal(Value::Bool(true))),
                "false" => Ok(Filter::Literal(Value::Bool(false))),
                "null" => Ok(Filter::Literal(Value::Null)),
                "nan" => Ok(Filter::Literal(Value::Number(number::nan_text()))),
                "infinite" => Ok(Filter::Literal(Value::Number(number::pos_inf()))),
                "not" => Ok(Filter::Not),
                "empty" => Ok(Filter::Empty),
                "length" => Ok(Filter::Length),
                "keys" => Ok(Filter::Keys),
                "has" => Ok(Filter::Has(self.parse_one_arg()?)),
                "select" => Ok(Filter::Select(self.parse_one_arg()?)),
                "map" => Ok(Filter::Map(self.parse_one_arg()?)),
                "type" => Ok(Filter::Type),
                "tonumber" => Ok(Filter::ToNumber),
                "tostring" => Ok(Filter::ToString),
                "join" => Ok(Filter::Join(self.parse_one_arg()?)),
                "ltrimstr" => Ok(Filter::LTrim(self.parse_one_arg()?)),
                "rtrimstr" => Ok(Filter::RTrim(self.parse_one_arg()?)),
                "todate" => Ok(Filter::ToDate),
                "fromdate" => Ok(Filter::FromDate),
                "gmtime" => Ok(Filter::GmTime),
                "localtime" => Ok(Filter::LocalTime),
                "mktime" => Ok(Filter::MkTime),
                "strftime" => Ok(Filter::StrfTime(self.parse_one_arg()?)),
                "strflocaltime" => Ok(Filter::StrfLocalTime(self.parse_one_arg()?)),
                "strptime" => Ok(Filter::StrpTime(self.parse_one_arg()?)),
                "now" => Ok(Filter::Now),
                "error" => self.parse_error(),
                "try" if !self.reject_block && !self.reject_try => self.parse_try(),
                "del" => Ok(Filter::Update {
                    op: UpdateOp::Modify,
                    path: self.parse_one_arg()?,
                    expr: Box::new(Filter::Empty),
                }),
                "path" => self.parse_fixed_call(at, "path", 1, |args| {
                    Filter::PathOf(args.into_iter().next().expect("one arg"))
                }),
                "paths" => self.parse_paths(at),
                "getpath" => self.parse_fixed_call(at, "getpath", 1, |args| {
                    Filter::GetPath(args.into_iter().next().expect("one arg"))
                }),
                "setpath" => self.parse_fixed_call(at, "setpath", 2, |args| {
                    let mut args = args.into_iter();
                    Filter::SetPath(args.next().expect("path"), args.next().expect("value"))
                }),
                "delpaths" => self.parse_fixed_call(at, "delpaths", 1, |args| {
                    Filter::DelPaths(args.into_iter().next().expect("one arg"))
                }),
                "if" if !self.reject_block => self.parse_if(),
                "reduce" if !self.reject_block => self.parse_fold(false),
                "foreach" if !self.reject_block => self.parse_fold(true),
                _ => Err(ProgramError::Syntax),
            };
        }
        Err(ProgramError::Syntax)
    }

    /// A math name, when `name` is one. `pow` with no call is `pow/0`.
    /// A call whose arguments parse, but not to the arity jq defined, is
    /// `name/n is not defined`. A call that does not parse is a syntax error.
    fn parse_math(&mut self, at: usize, name: &str) -> Result<Option<Filter>, ProgramError> {
        let Some(kind) = math::kind(name) else {
            return Ok(None);
        };
        self.skip_ws();
        if self.peek() != Some('(') {
            return match kind {
                Kind::Unary(op) => Ok(Some(Filter::Math1(op))),
                Kind::Abs => Ok(Some(Filter::Abs)),
                Kind::Pow10 => Ok(Some(Filter::Pow10)),
                Kind::Binary(_) | Kind::Fma => Err(self.undefined(at, name, 0)),
            };
        }
        let args = self.parse_call_args()?;
        let arity = args.len();
        match kind {
            Kind::Unary(_) | Kind::Abs | Kind::Pow10 => Err(self.undefined(at, name, arity)),
            Kind::Binary(op) if arity == 2 => {
                let mut args = args.into_iter();
                Ok(Some(Filter::Math2(
                    op,
                    args.next().expect("two args"),
                    args.next().expect("two args"),
                )))
            }
            Kind::Binary(_) => Err(self.undefined(at, name, arity)),
            Kind::Fma if arity == 3 => {
                let mut args = args.into_iter();
                Ok(Some(Filter::Math3(
                    args.next().expect("three args"),
                    args.next().expect("three args"),
                    args.next().expect("three args"),
                )))
            }
            Kind::Fma => Err(self.undefined(at, name, arity)),
        }
    }

    fn undefined(&self, at: usize, name: &str, arity: usize) -> ProgramError {
        report_at(self.src, at, &format!("{name}/{arity} is not defined"))
    }

    /// A call of exactly `arity` filters. No parentheses is `name/0`.
    fn parse_fixed_call(
        &mut self,
        at: usize,
        name: &str,
        arity: usize,
        build: impl FnOnce(Vec<Box<Filter>>) -> Filter,
    ) -> Result<Filter, ProgramError> {
        self.skip_ws();
        if self.peek() != Some('(') {
            return Err(self.undefined(at, name, 0));
        }
        let args = self.parse_call_args()?;
        if args.len() != arity {
            return Err(self.undefined(at, name, args.len()));
        }
        Ok(build(args))
    }

    /// `paths` takes no parentheses. `paths(expr)` takes one filter.
    fn parse_paths(&mut self, at: usize) -> Result<Filter, ProgramError> {
        self.skip_ws();
        if self.peek() != Some('(') {
            return Ok(Filter::Paths(None));
        }
        let args = self.parse_call_args()?;
        if args.len() != 1 {
            return Err(self.undefined(at, "paths", args.len()));
        }
        Ok(Filter::Paths(Some(
            args.into_iter().next().expect("one arg"),
        )))
    }

    /// Arguments of one call, separated by `;`. The count is how many
    /// expressions parsed, which is the arity jq reports.
    fn parse_call_args(&mut self) -> Result<Vec<Box<Filter>>, ProgramError> {
        if !self.eat('(') {
            return Err(ProgramError::Syntax);
        }
        let saved_block = self.reject_block;
        let saved_try = self.reject_try;
        self.reject_block = false;
        self.reject_try = false;
        let parsed = self.parse_separated_args();
        self.reject_block = saved_block;
        self.reject_try = saved_try;
        parsed
    }

    fn parse_separated_args(&mut self) -> Result<Vec<Box<Filter>>, ProgramError> {
        let mut args = vec![Box::new(self.parse_pipe()?)];
        while self.eat(';') {
            args.push(Box::new(self.parse_pipe()?));
        }
        if !self.eat(')') {
            return Err(ProgramError::Syntax);
        }
        Ok(args)
    }

    /// One filter argument, as in `has(expr)`. A second argument, separated
    /// by `;`, is rejected. `if`, `reduce`, and `foreach` are allowed inside.
    fn parse_one_arg(&mut self) -> Result<Box<Filter>, ProgramError> {
        self.skip_ws();
        if !self.eat('(') {
            return Err(ProgramError::Syntax);
        }
        let saved_block = self.reject_block;
        let saved_try = self.reject_try;
        self.reject_block = false;
        self.reject_try = false;
        let arg = self.parse_pipe();
        self.reject_block = saved_block;
        self.reject_try = saved_try;
        let arg = arg?;
        self.skip_ws();
        if self.peek() == Some(';') {
            return Err(ProgramError::Syntax);
        }
        if !self.eat(')') {
            return Err(ProgramError::Syntax);
        }
        Ok(Box::new(arg))
    }

    /// One juxtaposition term. `if`, `reduce`, and `foreach` need parentheses.
    fn parse_stream_term(&mut self) -> Result<Filter, ProgramError> {
        let saved = self.reject_block;
        self.reject_block = true;
        let term = self.parse_juxt();
        self.reject_block = saved;
        term
    }

    fn parse_fold(&mut self, foreach: bool) -> Result<Filter, ProgramError> {
        let stream = self.parse_stream_term()?;
        if !self.eat_keyword("as") {
            return Err(ProgramError::Syntax);
        }
        let pattern = self.parse_pattern()?;
        if !self.eat('(') {
            return Err(ProgramError::Syntax);
        }
        let init = self.parse_pipe()?;
        if !self.eat(';') {
            return Err(ProgramError::Syntax);
        }
        let update = self.parse_pipe()?;
        let extract = if foreach && self.eat(';') {
            Some(Box::new(self.parse_pipe()?))
        } else if !foreach && self.sees(';') {
            return Err(ProgramError::Syntax);
        } else {
            None
        };
        if !self.eat(')') {
            return Err(ProgramError::Syntax);
        }
        let stream = Box::new(stream);
        let init = Box::new(init);
        let update = Box::new(update);
        if foreach {
            Ok(Filter::Foreach {
                stream,
                pattern,
                init,
                update,
                extract,
            })
        } else {
            Ok(Filter::Reduce {
                stream,
                pattern,
                init,
                update,
            })
        }
    }

    fn parse_pattern(&mut self) -> Result<Pattern, ProgramError> {
        self.skip_ws();
        if self.eat('$') {
            let name = self.ident()?;
            return Ok(Pattern::Var(name));
        }
        if self.eat('[') {
            self.skip_ws();
            if self.peek() == Some(']') {
                return Err(ProgramError::Syntax);
            }
            let mut items = Vec::new();
            loop {
                items.push(self.parse_pattern()?);
                self.skip_ws();
                if self.eat(',') {
                    self.skip_ws();
                    if self.peek() == Some(']') {
                        return Err(ProgramError::Syntax);
                    }
                    continue;
                }
                if self.eat(']') {
                    break;
                }
                return Err(ProgramError::Syntax);
            }
            return Ok(Pattern::Array(items));
        }
        if self.eat('{') {
            self.skip_ws();
            if self.peek() == Some('}') {
                return Err(ProgramError::Syntax);
            }
            let mut fields = Vec::new();
            loop {
                fields.push(self.parse_pat_field()?);
                self.skip_ws();
                if self.eat(',') {
                    self.skip_ws();
                    if self.peek() == Some('}') {
                        return Err(ProgramError::Syntax);
                    }
                    continue;
                }
                if self.eat('}') {
                    break;
                }
                return Err(ProgramError::Syntax);
            }
            return Ok(Pattern::Object(fields));
        }
        Err(ProgramError::Syntax)
    }

    fn parse_pat_field(&mut self) -> Result<PatField, ProgramError> {
        self.skip_ws();
        if self.eat('$') {
            let name = self.ident()?;
            self.skip_ws();
            if self.eat(':') {
                let pattern = self.parse_pattern()?;
                return Ok(PatField {
                    key: Box::new(Filter::Literal(Value::String(name.clone()))),
                    pattern,
                    also: Some(name),
                });
            }
            return Ok(PatField {
                key: Box::new(Filter::Literal(Value::String(name.clone()))),
                pattern: Pattern::Var(name),
                also: None,
            });
        }
        if self.peek() == Some('(') {
            let pos = self.index;
            self.bump();
            let key = self.parse_pipe()?;
            self.skip_ws();
            if !self.eat(')') || !self.eat(':') {
                return Err(ProgramError::Syntax);
            }
            if let Some(constant) = const_one(&key) {
                if !matches!(constant, Value::String(_)) {
                    return Err(report_at(self.src, pos, &key_message(&constant)));
                }
            }
            let pattern = self.parse_pattern()?;
            return Ok(PatField {
                key: Box::new(key),
                pattern,
                also: None,
            });
        }
        if self.peek() == Some('"') {
            let key = self.parse_string_expr()?;
            if !self.eat(':') {
                return Err(ProgramError::Syntax);
            }
            let pattern = self.parse_pattern()?;
            return Ok(PatField {
                key: Box::new(key),
                pattern,
                also: None,
            });
        }
        if self.starts_ident() {
            let name = self.ident_key()?;
            if !self.eat(':') {
                return Err(ProgramError::Syntax);
            }
            let pattern = self.parse_pattern()?;
            return Ok(PatField {
                key: Box::new(Filter::Literal(Value::String(name))),
                pattern,
                also: None,
            });
        }
        Err(ProgramError::Syntax)
    }

    /// Pipe and unary minus, without comma or the arithmetic operators.
    /// Object values use this, so `{a:1+2}` stays a syntax error.
    /// `try` is rejected here. `(try ...)` and a call argument still accept it.
    fn parse_expd(&mut self) -> Result<Filter, ProgramError> {
        let saved = self.reject_try;
        self.reject_try = true;
        let parsed = self.parse_expd_pipe();
        self.reject_try = saved;
        parsed
    }

    fn parse_expd_pipe(&mut self) -> Result<Filter, ProgramError> {
        let mut filter = self.parse_expd_unary()?;
        while self.eat('|') {
            let right = self.parse_expd_unary()?;
            filter = Filter::Pipe(Box::new(filter), Box::new(right));
        }
        Ok(filter)
    }

    fn parse_error(&mut self) -> Result<Filter, ProgramError> {
        self.skip_ws();
        if self.peek() == Some('(') {
            Ok(Filter::Raise(self.parse_one_arg()?))
        } else {
            Ok(Filter::Error)
        }
    }

    /// `try` and `catch` stop before `+`, `*`, `|`, and `,`.
    /// A leading `-` still takes the product that follows it.
    fn parse_try(&mut self) -> Result<Filter, ProgramError> {
        let body = self.parse_try_operand()?;
        let handler = if self.eat_keyword("catch") {
            Some(Box::new(self.parse_try_operand()?))
        } else {
            None
        };
        Ok(Filter::Try {
            body: Box::new(body),
            handler,
        })
    }

    fn parse_try_operand(&mut self) -> Result<Filter, ProgramError> {
        self.skip_ws();
        if self.peek() == Some('-') && self.peek_at(1) != Some('=') {
            let saved = self.suffix_question;
            self.suffix_question = true;
            let operand = self.parse_signed();
            self.suffix_question = saved;
            return operand;
        }
        let primary = self.parse_primary()?;
        let filter = self.parse_try_tail(primary)?;
        if !self.eat_keyword("as") {
            return Ok(filter);
        }
        let pattern = self.parse_pattern()?;
        if !self.eat('|') {
            return Err(ProgramError::Syntax);
        }
        let body = self.parse_pipe()?;
        Ok(Filter::Bind {
            expr: Box::new(filter),
            pattern,
            body: Box::new(body),
        })
    }

    /// Indexing and fields after the primary. `?` applies only once a suffix
    /// has been written, and only to that last suffix.
    fn parse_try_tail(&mut self, mut filter: Filter) -> Result<Filter, ProgramError> {
        let mut extended = false;
        loop {
            self.skip_ws();
            if self.peek() == Some('[') {
                filter = self.parse_index(Some(filter))?;
                extended = true;
                continue;
            }
            if self.peek() == Some('.') {
                let bytes = self.src.as_bytes();
                if bytes.get(self.index + 1) == Some(&b'.') {
                    break;
                }
                self.bump();
                if self.peek() == Some('"') || self.spaced_quote() {
                    let key = self.parse_quoted_key()?;
                    filter = at_key(Some(filter), key);
                    extended = true;
                    continue;
                }
                if self.starts_ident() {
                    let name = self.ident()?;
                    filter = chain(Some(filter), Filter::Field(name));
                    extended = true;
                    continue;
                }
                if self.peek() == Some('[') {
                    filter = self.parse_index(Some(filter))?;
                    extended = true;
                    continue;
                }
                return Err(ProgramError::Syntax);
            }
            if extended && self.eat('?') {
                filter = optional_last(filter);
                continue;
            }
            break;
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
        let rest = &self.src[self.index..];
        // `//=` is an update, not an alternative followed by assignment.
        if rest.starts_with("//") && !rest.starts_with("//=") {
            self.index += 2;
            true
        } else {
            false
        }
    }

    fn take_update(&mut self) -> Option<UpdateOp> {
        self.skip_ws();
        let rest = &self.src[self.index..];
        let (op, len) = if rest.starts_with("|=") {
            (UpdateOp::Modify, 2)
        } else if rest.starts_with("+=") {
            (UpdateOp::Add, 2)
        } else if rest.starts_with("-=") {
            (UpdateOp::Sub, 2)
        } else if rest.starts_with("*=") {
            (UpdateOp::Mul, 2)
        } else if rest.starts_with("/=") {
            (UpdateOp::Div, 2)
        } else if rest.starts_with("%=") {
            (UpdateOp::Rem, 2)
        } else if rest.starts_with("//=") {
            (UpdateOp::Alt, 3)
        } else if rest.starts_with('=') && !rest.starts_with("==") {
            (UpdateOp::Assign, 1)
        } else {
            return None;
        };
        self.index += len;
        Some(op)
    }

    fn sees_update(&mut self) -> bool {
        let saved = self.index;
        let found = self.take_update().is_some();
        self.index = saved;
        found
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
        // `*=`, `/=`, and `%=` are updates.
        if rest.len() > 1
            && rest.as_bytes()[1] == b'='
            && matches!(rest.as_bytes()[0], b'*' | b'/' | b'%')
        {
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
                    let saved_try = self.reject_try;
                    self.reject_try = false;
                    let inner = self.parse_pipe();
                    self.reject_try = saved_try;
                    let inner = inner?;
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

    fn peek_at(&self, offset: usize) -> Option<char> {
        self.src[self.index..].chars().nth(offset)
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

pub(super) enum BracketTok {
    Num(String),
    Colon,
    Comma,
}

pub(super) fn filter_from_brackets(toks: Vec<BracketTok>) -> Result<Filter, ProgramError> {
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
                let text = canonical_number(&literal);
                let at = match array_index(&text) {
                    Some(index) => ArrayIndex::At(index),
                    None => ArrayIndex::OutOfRange,
                };
                items.push(Filter::Index { at, text });
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

pub(super) fn bracket_side(toks: &[BracketTok]) -> Result<Vec<Option<String>>, ProgramError> {
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

pub(super) fn combine_commas(items: Vec<Filter>) -> Filter {
    let mut items = items.into_iter();
    let mut filter = items.next().expect("one bracket term");
    for item in items {
        filter = Filter::Comma(Box::new(filter), Box::new(item));
    }
    filter
}

pub(super) fn chain(left: Option<Filter>, right: Filter) -> Filter {
    match left {
        Some(left) => Filter::Pipe(Box::new(left), Box::new(right)),
        None => right,
    }
}

pub(super) fn optional_last(filter: Filter) -> Filter {
    match filter {
        Filter::Pipe(left, right) => Filter::Pipe(left, Box::new(Filter::Optional(right))),
        // The outer Optional still swallows an error in the base or the key.
        // Only the index step itself keeps going.
        Filter::At { base, key, .. } => Filter::Optional(Box::new(Filter::At {
            base,
            key,
            optional: true,
        })),
        other => Filter::Optional(Box::new(other)),
    }
}

pub(super) fn cmp_at(rest: &str) -> Option<(Op, usize)> {
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

pub(super) fn at_key(base: Option<Filter>, key: Filter) -> Filter {
    if let Filter::Literal(Value::String(name)) = key {
        return chain(base, Filter::Field(name));
    }
    Filter::At {
        base: Box::new(base.unwrap_or(Filter::Identity)),
        key: Box::new(key),
        optional: false,
    }
}

/// The number text jq prints for a literal. Slice bounds stay raw in the
/// step so the slice math is unchanged, and `path` asks for this spelling.
pub(super) fn canonical_number(literal: &str) -> String {
    let literal = literal.strip_prefix('\u{1}').unwrap_or(literal);
    match crate::parse_stream(literal.as_bytes()) {
        Ok(parsed) if parsed.error.is_none() => match parsed.values.as_slice() {
            [one] => match &one.value {
                Value::Number(text) => return text.clone(),
                _ => {}
            },
            _ => {}
        },
        _ => {}
    }
    literal.to_string()
}

pub(super) fn number_value(token: &str) -> Result<Value, ProgramError> {
    let parsed = crate::parse_stream(token.as_bytes()).map_err(|_| ProgramError::Syntax)?;
    if parsed.error.is_some() {
        return Err(ProgramError::Syntax);
    }
    match parsed.values.as_slice() {
        [one] => Ok(one.value.clone()),
        _ => Err(ProgramError::Syntax),
    }
}

pub(super) fn decode_string_token(text: &str) -> Result<String, String> {
    crate::parse::decode_string_token(text)
}

pub(super) fn report_at(source: &str, pos: usize, message: &str) -> ProgramError {
    let (line_no, line, column) = line_at(source, pos);
    let mut text = format!("jq: error: {message} at <top-level>, line {line_no}:\n{line}");
    text.push_str(&" ".repeat(column));
    text.push_str("\njq: 1 compile error\n");
    ProgramError::Report(text)
}

pub(super) fn line_at(source: &str, pos: usize) -> (usize, String, usize) {
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

pub(super) fn key_message(value: &Value) -> String {
    format!(
        "Cannot use {} ({}) as object key",
        value.type_name(),
        shown(value)
    )
}

pub(super) fn const_one(filter: &Filter) -> Option<Value> {
    match const_outputs(filter)?.as_slice() {
        [one] => Some(one.clone()),
        _ => None,
    }
}

pub(super) fn const_outputs(filter: &Filter) -> Option<Vec<Value>> {
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

pub(super) fn is_identity(filter: &Filter) -> bool {
    matches!(filter, Filter::Identity)
}

/// `not(1)`, `length(1)`, and a second call such as `map(f)(1)`.
pub(super) fn forbids_call(filter: &Filter) -> bool {
    match filter {
        Filter::Not
        | Filter::Empty
        | Filter::Length
        | Filter::Keys
        | Filter::Has(_)
        | Filter::Select(_)
        | Filter::Map(_)
        | Filter::Type
        | Filter::ToNumber
        | Filter::ToString
        | Filter::Join(_)
        | Filter::LTrim(_)
        | Filter::RTrim(_)
        | Filter::ToDate
        | Filter::FromDate
        | Filter::GmTime
        | Filter::LocalTime
        | Filter::MkTime
        | Filter::StrfTime(_)
        | Filter::StrfLocalTime(_)
        | Filter::StrpTime(_)
        | Filter::Now
        | Filter::Math1(_)
        | Filter::Math2(_, _, _)
        | Filter::Math3(_, _, _)
        | Filter::Abs
        | Filter::Pow10
        | Filter::Error
        | Filter::Raise(_)
        | Filter::Try { .. }
        | Filter::PathOf(_)
        | Filter::Paths(_)
        | Filter::GetPath(_)
        | Filter::SetPath(_, _)
        | Filter::DelPaths(_) => true,
        Filter::Optional(inner) => forbids_call(inner),
        Filter::Pipe(_, right) => call_blocked(right),
        _ => false,
    }
}

/// A pipe forbids a following call only when its rightmost filter is
/// `error`, `error(expr)`, or `try`. Other pipes stay as they are.
pub(super) fn call_blocked(filter: &Filter) -> bool {
    match filter {
        Filter::Error | Filter::Raise(_) | Filter::Try { .. } => true,
        Filter::Optional(inner) | Filter::Pipe(_, inner) => call_blocked(inner),
        _ => false,
    }
}

pub(super) fn const_object(fields: &[ObjField]) -> Option<Value> {
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
