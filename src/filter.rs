//! Paths, slices, iteration, recurse, pipe, comma, and `?`.

use crate::number::{array_index, slice_bound};
use crate::parse;
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

pub fn compile(source: &str) -> Result<Filter, String> {
    let mut parser = Parser {
        src: source,
        index: 0,
    };
    parser.skip_ws();
    let filter = parser.parse_pipe()?;
    parser.skip_ws();
    if !parser.eof() {
        return Err("syntax error".to_string());
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
    fn parse_pipe(&mut self) -> Result<Filter, String> {
        let mut filter = self.parse_comma()?;
        while self.eat('|') {
            let right = self.parse_comma()?;
            filter = Filter::Pipe(Box::new(filter), Box::new(right));
        }
        Ok(filter)
    }

    fn parse_comma(&mut self) -> Result<Filter, String> {
        let mut filter = self.parse_juxt()?;
        while self.eat(',') {
            let right = self.parse_juxt()?;
            filter = Filter::Comma(Box::new(filter), Box::new(right));
        }
        Ok(filter)
    }

    fn parse_juxt(&mut self) -> Result<Filter, String> {
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

    fn parse_postfix(&mut self) -> Result<Filter, String> {
        let mut filter = self.parse_primary()?;
        self.skip_ws();
        while self.eat('?') {
            filter = Filter::Optional(Box::new(filter));
        }
        Ok(filter)
    }

    fn parse_primary(&mut self) -> Result<Filter, String> {
        self.skip_ws();
        if self.eat('(') {
            let inner = self.parse_pipe()?;
            self.skip_ws();
            if !self.eat(')') {
                return Err("syntax error".to_string());
            }
            return Ok(inner);
        }
        if self.peek() == Some('.') {
            return self.parse_path();
        }
        Err("syntax error".to_string())
    }

    fn parse_path(&mut self) -> Result<Filter, String> {
        self.bump();
        if self.peek() == Some('.') {
            self.bump();
            if self.peek() == Some('.') || self.starts_ident() {
                return Err("syntax error".to_string());
            }
            return self.finish_path(Some(Filter::Recurse));
        }
        let mut filter = None;
        if let Some(name) = self.quoted_field()? {
            filter = Some(Filter::Field(name));
        } else if self.starts_ident() {
            filter = Some(Filter::Field(self.ident()?));
        }
        self.finish_path(filter)
    }

    fn finish_path(&mut self, mut filter: Option<Filter>) -> Result<Filter, String> {
        loop {
            self.skip_ws();
            if self.eat('?') {
                let base = filter.unwrap_or(Filter::Identity);
                filter = Some(optional_last(base));
                continue;
            }
            self.skip_ws();
            if self.peek() == Some('[') {
                let segment = self.parse_brackets()?;
                filter = Some(chain(filter, segment));
                continue;
            }
            if self.peek() == Some('.') {
                let bytes = self.src.as_bytes();
                if bytes.get(self.index + 1) == Some(&b'.') {
                    return Err("syntax error".to_string());
                }
                self.bump();
                if let Some(name) = self.quoted_field()? {
                    filter = Some(chain(filter, Filter::Field(name)));
                    continue;
                }
                if self.starts_ident() {
                    let name = self.ident()?;
                    filter = Some(chain(filter, Filter::Field(name)));
                    continue;
                }
                if self.peek() == Some('[') {
                    let segment = self.parse_brackets()?;
                    filter = Some(chain(filter, segment));
                    continue;
                }
                return Err("syntax error".to_string());
            }
            break;
        }
        Ok(filter.unwrap_or(Filter::Identity))
    }

    /// A quoted name after `.`, including when a space sits between them.
    fn quoted_field(&mut self) -> Result<Option<String>, String> {
        if self.peek() == Some('"') {
            return Ok(Some(self.string()?));
        }
        let save = self.index;
        self.skip_ws();
        if self.peek() == Some('"') {
            return Ok(Some(self.string()?));
        }
        self.index = save;
        Ok(None)
    }

    fn parse_brackets(&mut self) -> Result<Filter, String> {
        if !self.eat('[') {
            return Err("syntax error".to_string());
        }
        self.skip_ws();
        if self.eat(']') {
            return Ok(Filter::Iterate);
        }
        let mut toks = Vec::new();
        loop {
            self.skip_ws();
            if self.peek() == Some('"') {
                toks.push(BracketTok::Str(self.string()?));
            } else if self
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
            return Err("syntax error".to_string());
        }
        filter_from_brackets(toks)
    }

    fn string(&mut self) -> Result<String, String> {
        let rest = &self.src[self.index..];
        let (value, consumed) =
            parse::parse_json_string_literal(rest).map_err(|_| "syntax error")?;
        self.index += consumed;
        Ok(value)
    }

    fn number_literal(&mut self) -> Result<&'a str, String> {
        let start = self.index;
        if self.peek() == Some('-') {
            self.bump();
        }
        if self.peek() == Some('.') {
            self.bump();
            if !self.take_digits() {
                return Err("syntax error".to_string());
            }
        } else if self.peek().is_some_and(|c| c.is_ascii_digit()) {
            self.take_digits();
            if self.peek() == Some('.') {
                self.bump();
                self.take_digits();
            }
        } else {
            return Err("syntax error".to_string());
        }
        if matches!(self.peek(), Some('e' | 'E')) {
            self.bump();
            if matches!(self.peek(), Some('+' | '-')) {
                self.bump();
            }
            if !self.take_digits() {
                return Err("syntax error".to_string());
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

    fn ident(&mut self) -> Result<String, String> {
        let start = self.index;
        if !self.starts_ident() {
            return Err("syntax error".to_string());
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
        while matches!(self.peek(), Some(' ' | '\t' | '\n' | '\r')) {
            self.bump();
        }
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
    Str(String),
    Colon,
    Comma,
}

fn filter_from_brackets(toks: Vec<BracketTok>) -> Result<Filter, String> {
    let colons = toks
        .iter()
        .filter(|tok| matches!(tok, BracketTok::Colon))
        .count();
    if colons > 1 {
        return Err("syntax error".to_string());
    }
    if colons == 1 {
        let at = toks
            .iter()
            .position(|tok| matches!(tok, BracketTok::Colon))
            .expect("colon");
        let starts = bracket_side(&toks[..at])?;
        let ends = bracket_side(&toks[at + 1..])?;
        if starts.iter().all(Option::is_none) && ends.iter().all(Option::is_none) {
            return Err("syntax error".to_string());
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
            BracketTok::Str(key) if expect_value => {
                items.push(Filter::Field(key));
                expect_value = false;
            }
            _ => return Err("syntax error".to_string()),
        }
    }
    if expect_value || items.is_empty() {
        return Err("syntax error".to_string());
    }
    Ok(combine_commas(items))
}

fn bracket_side(toks: &[BracketTok]) -> Result<Vec<Option<String>>, String> {
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
            _ => return Err("syntax error".to_string()),
        }
    }
    if expect_value || bounds.is_empty() {
        return Err("syntax error".to_string());
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
