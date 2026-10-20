//! Numbers, `true` / `false` / `null`, and the non-finite spellings jq accepts.

use super::parser::{is_delimiter, ParseErr, Parser};
use crate::number;
use crate::value::Value;

impl<'a> Parser<'a> {
    pub(in crate::parse) fn scan_atom(&mut self) -> Result<Value, ParseErr> {
        let start = self.index;
        if !self.nul_in_token() {
            let end = self.token_end();
            // `2` at the end of this file is not finished while another file
            // can still supply `3`.
            if self.open_ended && end == self.src.len() && end > start {
                self.consume_token(end);
                return Err(self.fail("Unfinished JSON term", true));
            }
            // Coordinates and other plain decimals are already in the form
            // jq prints. Copy those bytes. An exponent still goes through
            // the full normalization below.
            if let Some(text) = plain_number(&self.src[start..end]) {
                self.advance_ascii(end);
                return Ok(Value::Number(text));
            }
            self.consume_token(end);
            if let Some(value) = interpret_atom(&self.src[start..end]) {
                return Ok(value);
            }
            let phrase = invalid_phrase(&self.src[start..end]);
            return self.fail_atom(phrase);
        }
        // A NUL ends the token text. Junk after it, up to a delimiter or the
        // next file, is not part of the value. A number keeps the text before
        // the NUL. `true`, `false`, and `null` do not.
        let mut text = Vec::new();
        let mut saw_nul = false;
        loop {
            match self.peek() {
                None => break,
                Some(c) if is_delimiter(c) => break,
                Some('\0') => {
                    saw_nul = true;
                    self.bump();
                    while let Some(c) = self.peek() {
                        if is_delimiter(c) || self.at_boundary() {
                            break;
                        }
                        self.bump();
                    }
                    if self.peek().is_some_and(|c| !is_delimiter(c)) {
                        continue;
                    }
                    break;
                }
                Some(_) => {
                    let at = self.index;
                    self.bump();
                    text.extend_from_slice(&self.src[at..self.index]);
                }
            }
        }
        if text.is_empty() {
            return self.fail_atom("Invalid numeric literal");
        }
        if let Some(value) = interpret_atom(&text) {
            if saw_nul && !matches!(value, Value::Number(_)) {
                return self.fail_atom("Invalid literal");
            }
            return Ok(value);
        }
        self.fail_atom(invalid_phrase(&text))
    }

    fn fail_atom(&mut self, phrase: &'static str) -> Result<Value, ParseErr> {
        if self.eof() {
            Err(self.fail(phrase, true))
        } else {
            self.bump();
            Err(self.fail(phrase, false))
        }
    }

    /// Index of the delimiter that ends this token, or the end of the buffer.
    fn token_end(&self) -> usize {
        let bytes = self.src;
        let mut index = self.index;
        while index < bytes.len() && !is_delim_byte(bytes[index]) {
            index += 1;
        }
        index
    }

    fn consume_token(&mut self, end: usize) {
        while self.index < end {
            self.bump();
        }
    }

    /// A NUL sits in this token before the next delimiter.
    fn nul_in_token(&self) -> bool {
        let mut index = self.index;
        while index < self.src.len() {
            if self.boundaries.binary_search(&index).is_ok() && index != self.index {
                return false;
            }
            let byte = self.src[index];
            if byte == 0 {
                return true;
            }
            if matches!(
                byte,
                b' ' | b'\t' | b'\n' | b'\r' | b'"' | b'[' | b']' | b'{' | b'}' | b',' | b':'
            ) {
                return false;
            }
            index += 1;
        }
        false
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

    /// `nan` in any letter case. jq 1.7.1 on macOS then allows any digits,
    /// so `nan1234` is one number. jq 1.7 on Linux allows only zeros, so
    /// `nan0` is a number and `nan1234` is not. A letter after `nan` is
    /// not a number on either. `Infinity` takes no digits.
    fn parse_nan(&mut self) -> Result<Value, ()> {
        let rest = &self.src[self.index..];
        if rest.len() < 3 || !rest[..3].eq_ignore_ascii_case(b"nan") {
            return Err(());
        }
        self.index += 3;
        while self.nan_payload() {
            self.index += 1;
        }
        if !self.token_boundary_ok() {
            return Err(());
        }
        Ok(Value::Number(number::nan_text()))
    }

    fn nan_payload(&self) -> bool {
        match self.peek() {
            Some('0') => true,
            #[cfg(target_os = "macos")]
            Some('1'..='9') => true,
            _ => false,
        }
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

/// A number whose bytes are already what jq prints.
///
/// `0.0000001` and anything with an exponent are left out. Six leading
/// zeros in the fraction of `0.…` is where jq switches to an exponent.
fn plain_number(token: &[u8]) -> Option<String> {
    if token.is_empty() || token[0] == b'+' || token[0] == b'.' {
        return None;
    }
    let mut index = 0;
    if token[0] == b'-' {
        index = 1;
        if index >= token.len() {
            return None;
        }
    }
    if !token[index].is_ascii_digit() {
        return None;
    }
    let int_is_zero = token[index] == b'0';
    if int_is_zero {
        index += 1;
        if index < token.len() && token[index].is_ascii_digit() {
            return None;
        }
    } else {
        while index < token.len() && token[index].is_ascii_digit() {
            index += 1;
        }
    }
    if index < token.len() && token[index] == b'.' {
        index += 1;
        let frac = index;
        if index >= token.len() || !token[index].is_ascii_digit() {
            return None;
        }
        while index < token.len() && token[index].is_ascii_digit() {
            index += 1;
        }
        if int_is_zero {
            let mut zeros = 0;
            while frac + zeros < index && token[frac + zeros] == b'0' {
                zeros += 1;
            }
            if zeros >= 6 {
                return None;
            }
        }
    }
    if index != token.len() {
        return None;
    }
    std::str::from_utf8(token).ok().map(str::to_owned)
}

fn is_delim_byte(byte: u8) -> bool {
    matches!(
        byte,
        b' ' | b'\t' | b'\n' | b'\r' | b'"' | b'[' | b']' | b'{' | b'}' | b',' | b':'
    )
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
