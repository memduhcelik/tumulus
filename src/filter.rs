//! Paths, slices, iteration, recurse, pipe, comma, `?`, and values.

use crate::number::{array_index, slice_bound};
use crate::print::{self, Layout, Style};
use crate::value::Value;

#[derive(Clone, Debug)]
pub enum Filter {
    Identity,
    Field(String),
    Index(ArrayIndex),
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
    At {
        base: Box<Filter>,
        key: Box<Filter>,
    },
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

pub struct Failure {
    pub partial: Vec<Value>,
    pub message: String,
}

pub fn compile(source: &str) -> Result<Filter, ProgramError> {
    let mut parser = Parser {
        src: source,
        index: 0,
    };
    parser.skip_ws();
    let filter = parser.parse_pipe()?;
    parser.skip_ws();
    if !parser.eof() {
        return Err(ProgramError::Syntax);
    }
    Ok(filter)
}

pub fn eval(filter: &Filter, input: &Value) -> Result<Vec<Value>, Failure> {
    match filter {
        Filter::Identity => Ok(vec![input.clone()]),
        Filter::Field(key) => index_string(input, key),
        Filter::Index(index) => index_number(input, index),
        Filter::Slice { start, end } => slice(input, start.as_deref(), end.as_deref()),
        Filter::Iterate => iterate(input),
        Filter::Recurse => Ok(recurse(input)),
        Filter::Optional(inner) => match eval(inner, input) {
            Ok(values) => Ok(values),
            Err(failure) => Ok(failure.partial),
        },
        Filter::Pipe(left, right) => {
            let inputs = eval(left, input)?;
            let mut out = Vec::new();
            for value in inputs {
                match eval(right, &value) {
                    Ok(more) => out.extend(more),
                    Err(failure) => {
                        out.extend(failure.partial);
                        return Err(Failure {
                            partial: out,
                            message: failure.message,
                        });
                    }
                }
            }
            Ok(out)
        }
        Filter::Comma(left, right) => {
            let mut out = eval(left, input)?;
            match eval(right, input) {
                Ok(more) => out.extend(more),
                Err(failure) => {
                    out.extend(failure.partial);
                    return Err(Failure {
                        partial: out,
                        message: failure.message,
                    });
                }
            }
            Ok(out)
        }
        Filter::Literal(value) => Ok(vec![value.clone()]),
        Filter::Interpolate(pieces) => eval_interpolate(pieces, input),
        Filter::Collect(inner) => match eval(inner, input) {
            Ok(items) => Ok(vec![Value::Array(items)]),
            Err(failure) => Err(Failure {
                partial: Vec::new(),
                message: failure.message,
            }),
        },
        Filter::Object(fields) => eval_object(fields, input),
        Filter::At { base, key } => eval_at(base, key, input),
    }
}

fn index_string(input: &Value, key: &str) -> Result<Vec<Value>, Failure> {
    match input {
        Value::Null => Ok(vec![Value::Null]),
        Value::Object(items) => {
            let found = items
                .iter()
                .find(|(name, _)| name == key)
                .map(|(_, value)| value.clone())
                .unwrap_or(Value::Null);
            Ok(vec![found])
        }
        other => Err(fail(format!(
            "Cannot index {} with string \"{key}\"",
            other.type_name()
        ))),
    }
}

fn index_number(input: &Value, index: &ArrayIndex) -> Result<Vec<Value>, Failure> {
    match input {
        Value::Null => Ok(vec![Value::Null]),
        Value::Array(items) => {
            let value = match index {
                ArrayIndex::OutOfRange => Value::Null,
                ArrayIndex::At(index) => nth(items, *index),
            };
            Ok(vec![value])
        }
        other => Err(fail(format!(
            "Cannot index {} with number",
            other.type_name()
        ))),
    }
}

fn nth(items: &[Value], index: i64) -> Value {
    let len = items.len() as i128;
    let position = if index < 0 {
        len + i128::from(index)
    } else {
        i128::from(index)
    };
    if position < 0 || position >= len {
        Value::Null
    } else {
        items[position as usize].clone()
    }
}

fn slice(input: &Value, start: Option<&str>, end: Option<&str>) -> Result<Vec<Value>, Failure> {
    match input {
        Value::Null => Ok(vec![Value::Null]),
        Value::Array(items) => {
            let (from, to) = slice_range(start, end, items.len());
            Ok(vec![Value::Array(items[from..to].to_vec())])
        }
        Value::String(text) => {
            let chars: Vec<char> = text.chars().collect();
            let (from, to) = slice_range(start, end, chars.len());
            Ok(vec![Value::String(chars[from..to].iter().collect())])
        }
        other => Err(fail(format!(
            "Cannot index {} with object",
            other.type_name()
        ))),
    }
}

fn slice_range(start: Option<&str>, end: Option<&str>, len: usize) -> (usize, usize) {
    let len_i = i64::try_from(len).unwrap_or(i64::MAX);
    let from = match start {
        Some(literal) => slice_bound(literal, len_i, false),
        None => 0,
    };
    let to = match end {
        Some(literal) => slice_bound(literal, len_i, true),
        None => len_i,
    };
    let from = from.max(0) as usize;
    let to = to.max(0) as usize;
    let from = from.min(len);
    let to = to.min(len);
    if from >= to {
        (from, from)
    } else {
        (from, to)
    }
}

fn recurse(input: &Value) -> Vec<Value> {
    let mut out = Vec::new();
    let mut stack = vec![input.clone()];
    while let Some(value) = stack.pop() {
        if let Ok(mut children) = iterate(&value) {
            children.reverse();
            stack.extend(children);
        }
        out.push(value);
    }
    out
}

fn iterate(input: &Value) -> Result<Vec<Value>, Failure> {
    match input {
        Value::Array(items) => Ok(items.clone()),
        Value::Object(items) => Ok(items.iter().map(|(_, value)| value.clone()).collect()),
        other => Err(fail(format!(
            "Cannot iterate over {} ({})",
            other.type_name(),
            shown(other)
        ))),
    }
}

fn shown(value: &Value) -> String {
    let style = Style {
        layout: Layout::Compact,
        ..Style::default()
    };
    let rendered = print::render(value, style);
    if rendered.chars().count() > 14 {
        let mut truncated: String = rendered.chars().take(11).collect();
        truncated.push_str("...");
        truncated
    } else {
        rendered
    }
}

fn fail(message: String) -> Failure {
    Failure {
        partial: Vec::new(),
        message,
    }
}

struct Parser<'a> {
    src: &'a str,
    index: usize,
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
        let mut filter = self.parse_juxt()?;
        while self.eat(',') {
            let right = self.parse_juxt()?;
            filter = Filter::Comma(Box::new(filter), Box::new(right));
        }
        Ok(filter)
    }

    fn parse_juxt(&mut self) -> Result<Filter, ProgramError> {
        let mut filter = self.parse_postfix()?;
        loop {
            self.skip_ws();
            if matches!(self.peek(), Some('.' | '(')) {
                let right = self.parse_postfix()?;
                filter = Filter::Pipe(Box::new(filter), Box::new(right));
            } else {
                break;
            }
        }
        Ok(filter)
    }

    fn parse_postfix(&mut self) -> Result<Filter, ProgramError> {
        let mut filter = self.parse_unary()?;
        loop {
            self.skip_ws();
            if self.eat('?') {
                filter = Filter::Optional(Box::new(filter));
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

    /// A leading `-` is part of a number. `--1` is `1`, and `- 1` is `-1`.
    fn parse_unary(&mut self) -> Result<Filter, ProgramError> {
        self.skip_ws();
        let mut negative = false;
        while self.peek() == Some('-') {
            self.bump();
            negative = !negative;
            self.skip_ws();
        }
        let filter = self.parse_primary()?;
        if !negative {
            return Ok(filter);
        }
        match filter {
            Filter::Literal(Value::Number(text)) => {
                Ok(Filter::Literal(Value::Number(negate_number(&text))))
            }
            _ => Err(ProgramError::Syntax),
        }
    }

    fn parse_primary(&mut self) -> Result<Filter, ProgramError> {
        self.skip_ws();
        if self.eat('(') {
            let inner = self.parse_pipe()?;
            self.skip_ws();
            if !self.eat(')') {
                return Err(ProgramError::Syntax);
            }
            return Ok(inner);
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
            let name = self.ident()?;
            return match name.as_str() {
                "true" => Ok(Filter::Literal(Value::Bool(true))),
                "false" => Ok(Filter::Literal(Value::Bool(false))),
                "null" => Ok(Filter::Literal(Value::Null)),
                _ => Err(ProgramError::Syntax),
            };
        }
        Err(ProgramError::Syntax)
    }

    /// Pipe and unary minus, without the comma operator.
    /// Object values use this. A comma there separates fields.
    fn parse_expd(&mut self) -> Result<Filter, ProgramError> {
        let mut filter = self.parse_unary()?;
        while self.eat('|') {
            let right = self.parse_unary()?;
            filter = Filter::Pipe(Box::new(filter), Box::new(right));
        }
        Ok(filter)
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
                    let inner = self.parse_pipe()?;
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

    fn bump(&mut self) {
        if let Some(c) = self.peek() {
            self.index += c.len_utf8();
        }
    }

    fn eof(&self) -> bool {
        self.index >= self.src.len()
    }
}

enum BracketTok {
    Num(String),
    Colon,
    Comma,
}

fn filter_from_brackets(toks: Vec<BracketTok>) -> Result<Filter, ProgramError> {
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
                let index = match array_index(&literal) {
                    Some(index) => ArrayIndex::At(index),
                    None => ArrayIndex::OutOfRange,
                };
                items.push(Filter::Index(index));
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

fn bracket_side(toks: &[BracketTok]) -> Result<Vec<Option<String>>, ProgramError> {
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

fn combine_commas(items: Vec<Filter>) -> Filter {
    let mut items = items.into_iter();
    let mut filter = items.next().expect("one bracket term");
    for item in items {
        filter = Filter::Comma(Box::new(filter), Box::new(item));
    }
    filter
}

fn chain(left: Option<Filter>, right: Filter) -> Filter {
    match left {
        Some(left) => Filter::Pipe(Box::new(left), Box::new(right)),
        None => right,
    }
}

fn optional_last(filter: Filter) -> Filter {
    match filter {
        Filter::Pipe(left, right) => Filter::Pipe(left, Box::new(Filter::Optional(right))),
        other => Filter::Optional(Box::new(other)),
    }
}

fn at_key(base: Option<Filter>, key: Filter) -> Filter {
    if let Filter::Literal(Value::String(name)) = key {
        return chain(base, Filter::Field(name));
    }
    Filter::At {
        base: Box::new(base.unwrap_or(Filter::Identity)),
        key: Box::new(key),
    }
}

fn negate_number(text: &str) -> String {
    if text == "0" || text == "-0" {
        return "0".to_string();
    }
    match text.strip_prefix('-') {
        Some(rest) => rest.to_string(),
        None => format!("-{text}"),
    }
}

fn number_value(token: &str) -> Result<Value, ProgramError> {
    let parsed = crate::parse_stream(token.as_bytes()).map_err(|_| ProgramError::Syntax)?;
    if parsed.error.is_some() {
        return Err(ProgramError::Syntax);
    }
    match parsed.values.as_slice() {
        [one] => Ok(one.value.clone()),
        _ => Err(ProgramError::Syntax),
    }
}

fn decode_string_token(text: &str) -> Result<String, String> {
    crate::parse::decode_string_token(text)
}

fn report_at(source: &str, pos: usize, message: &str) -> ProgramError {
    let (line_no, line, column) = line_at(source, pos);
    let mut text = format!("jq: error: {message} at <top-level>, line {line_no}:\n{line}");
    text.push_str(&" ".repeat(column));
    text.push_str("\njq: 1 compile error\n");
    ProgramError::Report(text)
}

fn line_at(source: &str, pos: usize) -> (usize, String, usize) {
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

fn key_message(value: &Value) -> String {
    format!(
        "Cannot use {} ({}) as object key",
        value.type_name(),
        shown(value)
    )
}

fn const_one(filter: &Filter) -> Option<Value> {
    match const_outputs(filter)?.as_slice() {
        [one] => Some(one.clone()),
        _ => None,
    }
}

fn const_outputs(filter: &Filter) -> Option<Vec<Value>> {
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
        _ => None,
    }
}

fn is_identity(filter: &Filter) -> bool {
    matches!(filter, Filter::Identity)
}

fn const_object(fields: &[ObjField]) -> Option<Value> {
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

fn put(pairs: &mut Vec<(String, Value)>, key: String, value: Value) {
    if let Some((_, slot)) = pairs.iter_mut().find(|(name, _)| name == &key) {
        *slot = value;
    } else {
        pairs.push((key, value));
    }
}

fn eval_interpolate(pieces: &[Piece], input: &Value) -> Result<Vec<Value>, Failure> {
    let strings = expand_pieces(pieces, 0, String::new(), input)?;
    Ok(strings.into_iter().map(Value::String).collect())
}

fn expand_pieces(
    pieces: &[Piece],
    index: usize,
    prefix: String,
    input: &Value,
) -> Result<Vec<String>, Failure> {
    let Some(piece) = pieces.get(index) else {
        return Ok(vec![prefix]);
    };
    match piece {
        Piece::Text(text) => {
            let mut next = prefix;
            next.push_str(text);
            expand_pieces(pieces, index + 1, next, input)
        }
        Piece::Interp(filter) => match eval(filter, input) {
            Ok(values) => {
                let mut out = Vec::new();
                for value in values {
                    let mut next = prefix.clone();
                    next.push_str(&jq_text(&value));
                    out.extend(expand_pieces(pieces, index + 1, next, input)?);
                }
                Ok(out)
            }
            Err(failure) => {
                let mut partial = Vec::new();
                for value in failure.partial {
                    let mut next = prefix.clone();
                    next.push_str(&jq_text(&value));
                    match expand_pieces(pieces, index + 1, next, input) {
                        Ok(more) => partial.extend(more),
                        Err(inner) => {
                            for value in inner.partial {
                                if let Value::String(text) = value {
                                    partial.push(text);
                                }
                            }
                            return Err(Failure {
                                partial: partial.into_iter().map(Value::String).collect(),
                                message: failure.message,
                            });
                        }
                    }
                }
                Err(Failure {
                    partial: partial.into_iter().map(Value::String).collect(),
                    message: failure.message,
                })
            }
        },
    }
}

fn jq_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => {
            let style = Style {
                layout: Layout::Compact,
                ..Style::default()
            };
            print::render(other, style)
        }
    }
}

fn eval_object(fields: &[ObjField], input: &Value) -> Result<Vec<Value>, Failure> {
    let mut streams = Vec::with_capacity(fields.len());
    for field in fields {
        streams.push(field_alts(field, input));
    }
    combine_fields(&streams, 0, Vec::new())
}

struct Alts {
    pairs: Vec<(String, Value)>,
    error: Option<String>,
}

fn field_alts(field: &ObjField, input: &Value) -> Alts {
    match field {
        ObjField::Lookup(key) => lookup_alts(key, input),
        ObjField::Pair { key, value } => pair_alts(key, value, input),
    }
}

fn lookup_alts(key: &Filter, input: &Value) -> Alts {
    let found = match eval(key, input) {
        Ok(values) => (values, None),
        Err(failure) => (failure.partial, Some(failure.message)),
    };
    let mut pairs = Vec::new();
    for value in found.0 {
        let Value::String(name) = value else {
            return Alts {
                pairs,
                error: Some(key_message(&value)),
            };
        };
        match index_by(input, &Value::String(name.clone())) {
            Ok(got) => {
                let one = got.into_iter().next().unwrap_or(Value::Null);
                pairs.push((name, one));
            }
            Err(failure) => {
                return Alts {
                    pairs,
                    error: Some(failure.message),
                };
            }
        }
    }
    Alts {
        pairs,
        error: found.1,
    }
}

fn pair_alts(key: &Filter, value: &Filter, input: &Value) -> Alts {
    let keys = match eval(key, input) {
        Ok(values) => (values, None),
        Err(failure) => (failure.partial, Some(failure.message)),
    };
    let values = match eval(value, input) {
        Ok(values) => (values, None),
        Err(failure) => (failure.partial, Some(failure.message)),
    };
    let mut pairs = Vec::new();
    for key_value in &keys.0 {
        let Value::String(name) = key_value else {
            return Alts {
                pairs,
                error: Some(key_message(key_value)),
            };
        };
        for item in &values.0 {
            pairs.push((name.clone(), item.clone()));
        }
        if let Some(message) = &values.1 {
            return Alts {
                pairs,
                error: Some(message.clone()),
            };
        }
    }
    Alts {
        pairs,
        error: keys.1.or(values.1),
    }
}

fn combine_fields(
    streams: &[Alts],
    index: usize,
    base: Vec<(String, Value)>,
) -> Result<Vec<Value>, Failure> {
    let Some(stream) = streams.get(index) else {
        return Ok(vec![Value::Object(base)]);
    };
    let mut out = Vec::new();
    for (key, value) in &stream.pairs {
        let mut next = base.clone();
        put(&mut next, key.clone(), value.clone());
        match combine_fields(streams, index + 1, next) {
            Ok(more) => out.extend(more),
            Err(failure) => {
                out.extend(failure.partial);
                return Err(Failure {
                    partial: out,
                    message: failure.message,
                });
            }
        }
    }
    if let Some(message) = &stream.error {
        return Err(Failure {
            partial: out,
            message: message.clone(),
        });
    }
    Ok(out)
}

fn eval_at(base: &Filter, key: &Filter, input: &Value) -> Result<Vec<Value>, Failure> {
    let values = eval(base, input)?;
    let keys = match eval(key, input) {
        Ok(keys) => keys,
        Err(failure) => {
            let mut partial = Vec::new();
            for key in &failure.partial {
                for value in &values {
                    match index_by(value, key) {
                        Ok(got) => partial.extend(got),
                        Err(inner) => {
                            partial.extend(inner.partial);
                            return Err(Failure {
                                partial,
                                message: failure.message,
                            });
                        }
                    }
                }
            }
            return Err(Failure {
                partial,
                message: failure.message,
            });
        }
    };
    let mut out = Vec::new();
    for key in &keys {
        for value in &values {
            match index_by(value, key) {
                Ok(got) => out.extend(got),
                Err(failure) => {
                    out.extend(failure.partial);
                    return Err(Failure {
                        partial: out,
                        message: failure.message,
                    });
                }
            }
        }
    }
    Ok(out)
}

fn index_by(base: &Value, key: &Value) -> Result<Vec<Value>, Failure> {
    match key {
        Value::String(name) => index_string(base, name),
        Value::Number(text) => {
            let index = match array_index(text) {
                Some(index) => ArrayIndex::At(index),
                None => ArrayIndex::OutOfRange,
            };
            index_number(base, &index)
        }
        Value::Object(_) if matches!(base, Value::Null) => Ok(vec![Value::Null]),
        Value::Object(_) if matches!(base, Value::Array(_) | Value::String(_)) => Err(fail(
            "Array/string slice indices must be integers".to_string(),
        )),
        Value::Array(_) if matches!(base, Value::Array(_)) => Ok(vec![Value::Array(Vec::new())]),
        other => Err(fail(format!(
            "Cannot index {} with {}",
            base.type_name(),
            other.type_name()
        ))),
    }
}
