//! The first jq filters: paths, iteration, pipe, comma, and `?`.

use crate::number::array_index;
use crate::parse;
use crate::print::{self, Layout, Style};
use crate::value::Value;

#[derive(Clone, Debug)]
pub enum Filter {
    Identity,
    Field(String),
    Index(ArrayIndex),
    Iterate,
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
        Filter::Iterate => iterate(input),
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
            return Err("syntax error".to_string());
        }
        let mut filter = None;
        if self.starts_ident() {
            let name = self.ident()?;
            filter = Some(Filter::Field(name));
        }
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

    fn parse_brackets(&mut self) -> Result<Filter, String> {
        if !self.eat('[') {
            return Err("syntax error".to_string());
        }
        self.skip_ws();
        if self.eat(']') {
            return Ok(Filter::Iterate);
        }
        if self.peek() == Some('"') {
            let key = self.string()?;
            self.skip_ws();
            if !self.eat(']') {
                return Err("syntax error".to_string());
            }
            return Ok(Filter::Field(key));
        }
        let literal = self.number_literal()?;
        self.skip_ws();
        if !self.eat(']') {
            return Err("syntax error".to_string());
        }
        let index = match array_index(literal) {
            Some(index) => ArrayIndex::At(index),
            None => ArrayIndex::OutOfRange,
        };
        Ok(Filter::Index(index))
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
