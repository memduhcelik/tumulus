//! Numbers, `true` / `false` / `null`, and the non-finite spellings jq accepts.

use super::parser::{is_delimiter, ParseErr, Parser};
use crate::number;
use crate::value::Value;

impl<'a> Parser<'a> {
    pub(in crate::parse) fn scan_atom(&mut self) -> Result<Value, ParseErr> {
        let start = self.index;
        while self.peek().is_some_and(|c| !is_delimiter(c)) {
            self.bump();
        }
        let end = self.index;
        if let Some(value) = interpret_atom(&self.src[start..end]) {
            return Ok(value);
        }
        let phrase = invalid_phrase(&self.src[start..end]);
        if self.eof() {
            Err(self.fail(phrase, true))
        } else {
            self.bump();
            Err(self.fail(phrase, false))
        }
    }

    fn parse_scalar(&mut self) -> Result<Value, ()> {
        match self.peek() {
            Some('n' | 'N') => self.parse_n_word(),
            Some('i' | 'I') => self.parse_infinity(false),
            Some('t') => self.literal("true", Value::Bool(true)),
            Some('f') => self.literal("false", Value::Bool(false)),
            Some('+' | '-' | '.' | '0'..='9') => self.parse_number(),
            _ => Err(()),
        }
    }

    fn next_char(&mut self) -> Result<char, ()> {
        self.bump().ok_or(())
    }

    fn literal(&mut self, text: &str, value: Value) -> Result<Value, ()> {
        if !self.src[self.index..].starts_with(text.as_bytes()) {
            return Err(());
        }
        self.index += text.len();
        if !self.token_boundary_ok() {
            return Err(());
        }
        Ok(value)
    }

    fn token_boundary_ok(&self) -> bool {
        match self.peek() {
            None => true,
            Some(c) if is_delimiter(c) => true,
            Some(_) => false,
        }
    }

    fn parse_n_word(&mut self) -> Result<Value, ()> {
        if self.src[self.index..].starts_with(b"null") {
            return self.literal("null", Value::Null);
        }
        self.parse_nan()
    }

    /// `nan`, any letter case, then digits. `nan1234` is one number.
    /// A letter after `nan` is not a number. `Infinity` takes no digits.
    fn parse_nan(&mut self) -> Result<Value, ()> {
        let rest = &self.src[self.index..];
        if rest.len() < 3 || !rest[..3].eq_ignore_ascii_case(b"nan") {
            return Err(());
        }
        self.index += 3;
        while matches!(self.peek(), Some('0'..='9')) {
            self.index += 1;
        }
        if !self.token_boundary_ok() {
            return Err(());
        }
        Ok(Value::Number(number::nan_text()))
    }

    fn parse_infinity(&mut self, negative: bool) -> Result<Value, ()> {
        if self.take_nonfinite_word("infinity") || self.take_nonfinite_word("inf") {
            return Ok(Value::Number(number::infinite(negative)));
        }
        Err(())
    }

    fn take_nonfinite_word(&mut self, word: &str) -> bool {
        let rest = &self.src[self.index..];
        if rest.len() < word.len() || !rest[..word.len()].eq_ignore_ascii_case(word.as_bytes()) {
            return false;
        }
        let after = self.index + word.len();
        let boundary = match self.char_at(after) {
            None => true,
            Some(c) if is_delimiter(c) => true,
            Some(_) => false,
        };
        if !boundary {
            return false;
        }
        self.index = after;
        true
    }

    fn parse_number(&mut self) -> Result<Value, ()> {
        let mut negative = false;
        match self.peek() {
            Some('+') => {
                self.next_char()?;
            }
            Some('-') => {
                self.next_char()?;
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
            self.next_char()?;
            let frac = self.take_digits();
            if frac.is_empty() {
                return Err(());
            }
            frac_digits = Some(frac);
        } else if matches!(self.peek(), Some('0'..='9')) {
            int_digits = self.take_digits();
            if self.peek() == Some('.') {
                self.next_char()?;
                frac_digits = Some(self.take_digits());
            }
        } else {
            return Err(());
        }

        let mut exponent = None;
        if matches!(self.peek(), Some('e' | 'E')) {
            self.next_char()?;
            let mut exp_negative = false;
            match self.peek() {
                Some('+') => {
                    self.next_char()?;
                }
                Some('-') => {
                    self.next_char()?;
                    exp_negative = true;
                }
                _ => {}
            }
            let digits = self.take_digits();
            if digits.is_empty() {
                return Err(());
            }
            exponent = Some((exp_negative, digits));
        }
        if !self.token_boundary_ok() {
            return Err(());
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
            self.bump();
        }
        digits
    }
}

fn invalid_phrase(token: &[u8]) -> &'static str {
    // `t` and `f` start true/false. `nu` has already chosen null over nan.
    let literal = token.starts_with(b"t") || token.starts_with(b"f") || token.starts_with(b"nu");
    if literal {
        "Invalid literal"
    } else {
        "Invalid numeric literal"
    }
}

fn interpret_atom(token: &[u8]) -> Option<Value> {
    let mut parser = Parser::new(token);
    let value = parser.parse_scalar().ok()?;
    if parser.eof() {
        Some(value)
    } else {
        None
    }
}
