use std::io::Write;

use crate::error::Error;
use crate::number;
use crate::print::{self, Style};
use crate::value::Value;

/// Parse every JSON value in `bytes` and write jq's layout for each one.
///
/// A leading UTF-8 BOM is ignored. Numbers are rewritten the way jq rewrites them.
pub struct LocatedValue {
    pub value: Value,
    /// Line number jq prints in a runtime error for this value.
    pub line: usize,
}

pub fn write_input(bytes: &[u8], style: Style, out: &mut dyn Write) -> Result<(), Error> {
    for item in parse_stream(bytes)? {
        let rendered = print::render(&item.value, style);
        out.write_all(rendered.as_bytes())?;
        if !style.join {
            out.write_all(b"\n")?;
        }
    }
    Ok(())
}

pub fn parse_stream(bytes: &[u8]) -> Result<Vec<LocatedValue>, Error> {
    let bytes = strip_bom(bytes);
    let text = std::str::from_utf8(bytes).map_err(|_| Error::new("input is not utf-8"))?;
    let mut parser = Parser::new(text);
    let mut parsed = Vec::new();
    loop {
        parser.skip_ws();
        if parser.eof() {
            break;
        }
        let start_line = parser.line;
        let value = parser.parse_value()?;
        parsed.push((value, start_line, parser.index));
    }
    let last = parsed.len();
    let mut out = Vec::with_capacity(last);
    for (index, (value, start_line, end)) in parsed.into_iter().enumerate() {
        let newline_after = text[end..].contains('\n');
        let line = if index + 1 == last && !newline_after {
            start_line.saturating_sub(1)
        } else {
            start_line
        };
        out.push(LocatedValue { value, line });
    }
    Ok(out)
}

pub(crate) fn parse_json_string_literal(text: &str) -> Result<(String, usize), Error> {
    let mut parser = Parser::new(text);
    let value = parser.parse_string()?;
    Ok((value, parser.index))
}

fn strip_bom(bytes: &[u8]) -> &[u8] {
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        &bytes[3..]
    } else {
        bytes
    }
}

struct Parser<'a> {
    src: &'a str,
    index: usize,
    line: usize,
    column: usize,
}

impl<'a> Parser<'a> {
    fn new(src: &'a str) -> Self {
        Self {
            src,
            index: 0,
            line: 1,
            column: 1,
        }
    }

    fn eof(&self) -> bool {
        self.index >= self.src.len()
    }

    fn peek(&self) -> Option<char> {
        self.src[self.index..].chars().next()
    }

    fn bump(&mut self) -> Result<char, Error> {
        let mut chars = self.src[self.index..].chars();
        let Some(c) = chars.next() else {
            return Err(self.error("unexpected end of input"));
        };
        self.index += c.len_utf8();
        if c == '\n' {
            self.line += 1;
            self.column = 1;
        } else {
            self.column += 1;
        }
        Ok(c)
    }

    fn skip_ws(&mut self) {
        while matches!(self.peek(), Some(' ' | '\t' | '\n' | '\r')) {
            let _ = self.bump();
        }
    }

    fn error(&self, message: &str) -> Error {
        Error::new(format!(
            "{message} at line {}, column {}",
            self.line, self.column
        ))
    }

    fn parse_value(&mut self) -> Result<Value, Error> {
        match self.peek() {
            Some('n' | 'N') => self.parse_n_word(),
            Some('i' | 'I') => self.parse_infinity(false),
            Some('t') => self.literal("true", Value::Bool(true)),
            Some('f') => self.literal("false", Value::Bool(false)),
            Some('"') => Ok(Value::String(self.parse_string()?)),
            Some('[') => {
                self.bump()?;
                self.parse_array()
            }
            Some('{') => {
                self.bump()?;
                self.parse_object()
            }
            Some('+' | '-' | '.' | '0'..='9') => self.parse_number(),
            Some(_) => Err(self.error("expected a value")),
            None => Err(self.error("expected a value")),
        }
    }

    fn literal(&mut self, text: &str, value: Value) -> Result<Value, Error> {
        if !self.src[self.index..].starts_with(text) {
            return Err(self.error("invalid literal"));
        }
        let after = self.index + text.len();
        // Reject the keyword before producing a value when the next character
        // still belongs to the token (`truefalse`, `true-1`).
        self.index = after;
        let saved_column = self.column;
        self.column += text.len();
        if !self.token_boundary_ok() {
            self.index = after - text.len();
            self.column = saved_column;
            return Err(self.error("invalid literal"));
        }
        Ok(value)
    }

    fn token_boundary_ok(&self) -> bool {
        match self.peek() {
            None => true,
            Some(' ' | '\t' | '\n' | '\r' | '"' | '[' | '{' | ']' | '}' | ',' | ':') => true,
            Some(_) => false,
        }
    }

    fn parse_array(&mut self) -> Result<Value, Error> {
        self.skip_ws();
        let mut items = Vec::new();
        if self.peek() == Some(']') {
            self.bump()?;
            return Ok(Value::Array(items));
        }
        loop {
            items.push(self.parse_value()?);
            self.skip_ws();
            match self.peek() {
                Some(',') => {
                    self.bump()?;
                    self.skip_ws();
                }
                Some(']') => {
                    self.bump()?;
                    break;
                }
                _ => return Err(self.error("expected ',' or ']'")),
            }
        }
        Ok(Value::Array(items))
    }

    fn parse_object(&mut self) -> Result<Value, Error> {
        self.skip_ws();
        // First key position wins; a later duplicate replaces the value.
        let mut items: Vec<(String, Value)> = Vec::new();
        if self.peek() == Some('}') {
            self.bump()?;
            return Ok(Value::Object(items));
        }
        loop {
            if self.peek() != Some('"') {
                return Err(self.error("expected a string key"));
            }
            let key = self.parse_string()?;
            self.skip_ws();
            if self.peek() != Some(':') {
                return Err(self.error("expected ':'"));
            }
            self.bump()?;
            self.skip_ws();
            let value = self.parse_value()?;
            if let Some((_, slot)) = items.iter_mut().find(|(existing, _)| existing == &key) {
                *slot = value;
            } else {
                items.push((key, value));
            }
            self.skip_ws();
            match self.peek() {
                Some(',') => {
                    self.bump()?;
                    self.skip_ws();
                }
                Some('}') => {
                    self.bump()?;
                    break;
                }
                _ => return Err(self.error("expected ',' or '}'")),
            }
        }
        Ok(Value::Object(items))
    }

    fn parse_string(&mut self) -> Result<String, Error> {
        if self.bump()? != '"' {
            return Err(self.error("expected a string"));
        }
        let mut out = String::new();
        loop {
            let c = self.bump()?;
            match c {
                '"' => return Ok(out),
                '\\' => {
                    let escaped = self.bump()?;
                    match escaped {
                        '"' | '\\' | '/' => out.push(escaped),
                        'b' => out.push('\u{0008}'),
                        'f' => out.push('\u{000c}'),
                        'n' => out.push('\n'),
                        'r' => out.push('\r'),
                        't' => out.push('\t'),
                        'u' => {
                            let unit = self.hex4()?;
                            self.push_unicode(&mut out, unit)?;
                        }
                        _ => return Err(self.error("invalid escape")),
                    }
                }
                c if (c as u32) < 0x20 => {
                    return Err(self.error("unescaped control character in string"));
                }
                c => out.push(c),
            }
        }
    }

    fn push_unicode(&mut self, out: &mut String, unit: u32) -> Result<(), Error> {
        if (0xD800..=0xDBFF).contains(&unit) {
            if self.bump()? != '\\' || self.bump()? != 'u' {
                return Err(self.error("invalid surrogate pair"));
            }
            let low = self.hex4()?;
            if !(0xDC00..=0xDFFF).contains(&low) {
                return Err(self.error("invalid surrogate pair"));
            }
            let scalar = 0x10000 + (((unit - 0xD800) << 10) | (low - 0xDC00));
            let Some(c) = char::from_u32(scalar) else {
                return Err(self.error("invalid surrogate pair"));
            };
            out.push(c);
            return Ok(());
        }
        if (0xDC00..=0xDFFF).contains(&unit) {
            return Err(self.error("invalid surrogate pair"));
        }
        let Some(c) = char::from_u32(unit) else {
            return Err(self.error("invalid unicode escape"));
        };
        out.push(c);
        Ok(())
    }

    fn hex4(&mut self) -> Result<u32, Error> {
        let mut value = 0u32;
        for _ in 0..4 {
            let c = self.bump()?;
            let Some(digit) = c.to_digit(16) else {
                return Err(self.error("invalid hex escape"));
            };
            value = (value << 4) | digit;
        }
        Ok(value)
    }

    fn parse_n_word(&mut self) -> Result<Value, Error> {
        if self.src[self.index..].starts_with("null") {
            return self.literal("null", Value::Null);
        }
        self.parse_nan()
    }

    fn parse_nan(&mut self) -> Result<Value, Error> {
        if self.take_nonfinite_word("nan") {
            // jq keeps NaN as a number whose text is null.
            return Ok(Value::Number("null".to_string()));
        }
        Err(self.error("invalid number"))
    }

    fn parse_infinity(&mut self, negative: bool) -> Result<Value, Error> {
        if self.take_nonfinite_word("infinity") || self.take_nonfinite_word("inf") {
            return Ok(Value::Number(number::infinite(negative)));
        }
        Err(self.error("invalid number"))
    }

    /// Case-insensitive `nan`, `inf`, or `infinity`, only when the next
    /// character ends the token the way jq ends it.
    fn take_nonfinite_word(&mut self, word: &str) -> bool {
        let rest = &self.src[self.index..];
        let bytes = rest.as_bytes();
        let pat = word.as_bytes();
        if bytes.len() < pat.len() || !bytes[..pat.len()].eq_ignore_ascii_case(pat) {
            return false;
        }
        let after = self.index + pat.len();
        let boundary = match self.src[after..].chars().next() {
            None => true,
            Some(' ' | '\t' | '\n' | '\r' | '"' | '[' | '{' | ']' | '}' | ',' | ':') => true,
            Some(_) => false,
        };
        if !boundary {
            return false;
        }
        self.index = after;
        self.column += pat.len();
        true
    }

    fn parse_number(&mut self) -> Result<Value, Error> {
        let mut negative = false;
        match self.peek() {
            Some('+') => {
                self.bump()?;
            }
            Some('-') => {
                self.bump()?;
                negative = true;
            }
            _ => {}
        }

        if matches!(self.peek(), Some('i' | 'I')) {
            return self.parse_infinity(negative);
        }
        if matches!(self.peek(), Some('n' | 'N')) {
            return self.parse_nan();
        }

        let mut int_digits = String::new();
        let mut frac_digits = None;
        if self.peek() == Some('.') {
            self.bump()?;
            let frac = self.take_digits();
            if frac.is_empty() {
                return Err(self.error("invalid number"));
            }
            frac_digits = Some(frac);
        } else if matches!(self.peek(), Some('0'..='9')) {
            int_digits = self.take_digits();
            if self.peek() == Some('.') {
                self.bump()?;
                frac_digits = Some(self.take_digits());
            }
        } else {
            return Err(self.error("invalid number"));
        }

        let mut exponent = None;
        if matches!(self.peek(), Some('e' | 'E')) {
            self.bump()?;
            let mut exp_negative = false;
            match self.peek() {
                Some('+') => {
                    self.bump()?;
                }
                Some('-') => {
                    self.bump()?;
                    exp_negative = true;
                }
                _ => {}
            }
            let digits = self.take_digits();
            if digits.is_empty() {
                return Err(self.error("invalid number"));
            }
            exponent = Some((exp_negative, digits));
        }
        if !self.token_boundary_ok() {
            return Err(self.error("invalid number"));
        }
        let text = number::normalize(
            negative,
            &int_digits,
            frac_digits.as_deref(),
            exponent
                .as_ref()
                .map(|(exp_negative, digits)| (*exp_negative, digits.as_str())),
        );
        Ok(Value::Number(text))
    }

    fn take_digits(&mut self) -> String {
        let mut digits = String::new();
        while let Some(digit @ '0'..='9') = self.peek() {
            digits.push(digit);
            let _ = self.bump();
        }
        digits
    }
}
