use std::io::Write;

use crate::error::Error;
use crate::number;
use crate::print::{self, Style};
use crate::value::Value;

/// One JSON value and the line jq would name in a runtime error.
pub struct LocatedValue {
    pub value: Value,
    /// Line number jq prints in a runtime error for this value.
    pub line: usize,
}

/// Values read from one input, plus a parse error if the tail is not valid JSON.
///
/// Complete values that ended before the bad token are kept. jq prints those
/// before it reports the parse error.
pub struct ParsedStream {
    pub values: Vec<LocatedValue>,
    pub error: Option<String>,
}

/// Parse every JSON value in `bytes` and write jq's layout for each one.
///
/// A leading UTF-8 BOM is ignored. Numbers are rewritten the way jq rewrites them.
/// When the tail is truncated, values before that tail are still written.
pub fn write_input(bytes: &[u8], style: Style, out: &mut dyn Write) -> Result<(), Error> {
    let parsed = parse_stream(bytes)?;
    for item in &parsed.values {
        let rendered = print::render(&item.value, style);
        out.write_all(rendered.as_bytes())?;
        if !style.join {
            out.write_all(b"\n")?;
        }
    }
    if let Some(message) = parsed.error {
        return Err(Error::new(message));
    }
    Ok(())
}

pub fn parse_stream(bytes: &[u8]) -> Result<ParsedStream, Error> {
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
        let value = match parser.parse_value() {
            Ok(value) => value,
            Err(err) => {
                return Ok(ParsedStream {
                    values: locate(text, parsed),
                    error: Some(err.text()),
                });
            }
        };
        let end = parser.index;
        // A number or literal glued to ] } , : is not a finished value.
        // jq ends the token on that character and then rejects it.
        if is_atom(&value) {
            let phrase = match parser.peek() {
                Some(']') => Some("Unmatched ']'"),
                Some('}') => Some("Unmatched '}'"),
                Some(',') => Some("Expected value before ','"),
                Some(':') => Some("Expected string key before ':'"),
                _ => None,
            };
            if let Some(phrase) = phrase {
                parser.bump();
                return Ok(ParsedStream {
                    values: locate(text, parsed),
                    error: Some(parser.fail(phrase, false).text()),
                });
            }
        }
        parsed.push((value, start_line, end));
    }
    Ok(ParsedStream {
        values: locate(text, parsed),
        error: None,
    })
}

pub(crate) fn parse_json_string_literal(text: &str) -> Result<(String, usize), Error> {
    let mut parser = Parser::new(text);
    let value = parser
        .parse_string()
        .map_err(|_| Error::new("bad string"))?;
    Ok((value, parser.index))
}

fn locate(text: &str, parsed: Vec<(Value, usize, usize)>) -> Vec<LocatedValue> {
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
    out
}

fn strip_bom(bytes: &[u8]) -> &[u8] {
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        &bytes[3..]
    } else {
        bytes
    }
}

fn is_atom(value: &Value) -> bool {
    matches!(value, Value::Null | Value::Bool(_) | Value::Number(_))
}

fn is_ws(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\r')
}

fn is_delimiter(c: char) -> bool {
    is_ws(c) || matches!(c, '"' | '[' | ']' | '{' | '}' | ',' | ':')
}

struct ParseErr {
    phrase: &'static str,
    at_eof: bool,
    line: usize,
    column: usize,
}

impl ParseErr {
    fn text(&self) -> String {
        if self.at_eof {
            format!(
                "{} at EOF at line {}, column {}",
                self.phrase, self.line, self.column
            )
        } else {
            format!(
                "{} at line {}, column {}",
                self.phrase, self.line, self.column
            )
        }
    }
}

struct Parser<'a> {
    src: &'a str,
    index: usize,
    /// jq counts the line from 1.
    line: usize,
    /// jq counts bytes, starts at 0, and returns to 0 after a newline.
    column: usize,
    /// Nesting of arrays and objects. jq rejects the 257th level.
    depth: usize,
}

impl<'a> Parser<'a> {
    fn new(src: &'a str) -> Self {
        Self {
            src,
            index: 0,
            line: 1,
            column: 0,
            depth: 0,
        }
    }

    fn eof(&self) -> bool {
        self.index >= self.src.len()
    }

    fn peek(&self) -> Option<char> {
        self.src[self.index..].chars().next()
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.index += c.len_utf8();
        if c == '\n' {
            self.line += 1;
            self.column = 0;
        } else {
            self.column += c.len_utf8();
        }
        Some(c)
    }

    fn skip_ws(&mut self) {
        while self.peek().is_some_and(is_ws) {
            self.bump();
        }
    }

    fn fail(&self, phrase: &'static str, at_eof: bool) -> ParseErr {
        ParseErr {
            phrase,
            at_eof,
            line: self.line,
            column: self.column,
        }
    }

    fn bump_fail(&mut self, phrase: &'static str) -> ParseErr {
        self.bump();
        self.fail(phrase, false)
    }

    fn parse_value(&mut self) -> Result<Value, ParseErr> {
        match self.peek() {
            None => Err(self.fail("Unfinished JSON term", true)),
            Some('"') => Ok(Value::String(self.parse_string()?)),
            Some('[') => self.parse_container(true),
            Some('{') => self.parse_container(false),
            Some(']') => Err(self.bump_fail("Unmatched ']'")),
            Some('}') => Err(self.bump_fail("Unmatched '}'")),
            Some(',') => Err(self.bump_fail("Expected value before ','")),
            Some(':') => Err(self.bump_fail("Expected string key before ':'")),
            Some(_) => self.scan_atom(),
        }
    }

    fn parse_container(&mut self, array: bool) -> Result<Value, ParseErr> {
        self.bump();
        if self.depth >= 256 {
            return Err(self.fail("Exceeds depth limit for parsing", false));
        }
        self.depth += 1;
        let value = if array {
            self.parse_array()
        } else {
            self.parse_object()
        };
        self.depth -= 1;
        value
    }

    fn parse_array(&mut self) -> Result<Value, ParseErr> {
        self.skip_ws();
        if self.peek() == Some(']') {
            self.bump();
            return Ok(Value::Array(Vec::new()));
        }
        let mut items = Vec::new();
        loop {
            match self.peek() {
                None => return Err(self.fail("Unfinished JSON term", true)),
                Some(']') => return Err(self.bump_fail("Expected another array element")),
                Some('}') => return Err(self.bump_fail("Unmatched '}'")),
                Some(',') => return Err(self.bump_fail("Expected value before ','")),
                Some(_) => items.push(self.parse_value()?),
            }
            self.skip_ws();
            match self.peek() {
                Some(',') => {
                    self.bump();
                    self.skip_ws();
                }
                Some(']') => {
                    self.bump();
                    break;
                }
                None => return Err(self.fail("Unfinished JSON term", true)),
                Some('}') => return Err(self.bump_fail("Unmatched '}'")),
                Some(':') => return Err(self.bump_fail("':' not as part of an object")),
                Some(_) => return Err(self.expected_separator()),
            }
        }
        Ok(Value::Array(items))
    }

    fn parse_object(&mut self) -> Result<Value, ParseErr> {
        self.skip_ws();
        if self.peek() == Some('}') {
            self.bump();
            return Ok(Value::Object(Vec::new()));
        }
        // First key position wins; a later duplicate replaces the value.
        let mut items: Vec<(String, Value)> = Vec::new();
        loop {
            let key = match self.peek() {
                None => return Err(self.fail("Unfinished JSON term", true)),
                Some('}') => return Err(self.bump_fail("Expected another key-value pair")),
                Some(']') => return Err(self.bump_fail("Unmatched ']'")),
                Some(',') => return Err(self.bump_fail("Expected value before ','")),
                Some(':') => return Err(self.bump_fail("Expected string key before ':'")),
                Some('"') => {
                    let key = self.parse_string()?;
                    self.expect_colon()?;
                    key
                }
                Some(_) => {
                    self.parse_value()?;
                    return Err(self.bad_key());
                }
            };
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
                    self.bump();
                    self.skip_ws();
                }
                Some('}') => {
                    self.bump();
                    break;
                }
                None => return Err(self.fail("Unfinished JSON term", true)),
                Some(']') => return Err(self.bump_fail("Unmatched ']'")),
                Some(':') => return Err(self.bump_fail("':' not as part of an object")),
                Some(_) => return Err(self.expected_separator()),
            }
        }
        Ok(Value::Object(items))
    }

    fn expect_colon(&mut self) -> Result<(), ParseErr> {
        self.skip_ws();
        match self.peek() {
            Some(':') => {
                self.bump();
                Ok(())
            }
            Some('}' | ',') => Err(self.bump_fail("Objects must consist of key:value pairs")),
            Some(']') => Err(self.bump_fail("Unmatched ']'")),
            None => Err(self.fail("Unfinished JSON term", true)),
            Some(_) => Err(self.expected_separator()),
        }
    }

    fn bad_key(&mut self) -> ParseErr {
        self.skip_ws();
        match self.peek() {
            Some(':') => self.bump_fail("Object keys must be strings"),
            Some('}' | ',') => self.bump_fail("Objects must consist of key:value pairs"),
            Some(']') => self.bump_fail("Unmatched ']'"),
            None => self.fail("Unfinished JSON term", true),
            Some(_) => self.expected_separator(),
        }
    }

    /// A value showed up where a comma was required. Point at the end of that
    /// value: the closing quote of a string, the opening bracket of a container,
    /// or the byte that ends an atom.
    fn expected_separator(&mut self) -> ParseErr {
        match self.peek() {
            Some('[' | '{') => {
                self.bump();
                self.fail("Expected separator between values", false)
            }
            Some('"') => match self.parse_string() {
                Err(err) => err,
                Ok(_) => self.fail("Expected separator between values", false),
            },
            Some(_) => match self.scan_atom() {
                Err(err) => err,
                Ok(_) => {
                    if self.eof() {
                        self.fail("Expected separator between values", true)
                    } else {
                        self.bump();
                        self.fail("Expected separator between values", false)
                    }
                }
            },
            None => self.fail("Unfinished JSON term", true),
        }
    }

    fn scan_atom(&mut self) -> Result<Value, ParseErr> {
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

    fn parse_string(&mut self) -> Result<String, ParseErr> {
        self.bump();
        let mut out = String::new();
        let mut problem = None;
        loop {
            let Some(c) = self.bump() else {
                return Err(self.fail("Unfinished string", true));
            };
            match c {
                '"' => {
                    if let Some(phrase) = problem {
                        return Err(self.fail(phrase, false));
                    }
                    return Ok(out);
                }
                '\\' => self.parse_escape(&mut out, &mut problem)?,
                c if (c as u32) < 0x20 => note(&mut problem, CONTROL),
                c => {
                    if problem.is_none() {
                        out.push(c);
                    }
                }
            }
        }
    }

    fn parse_escape(
        &mut self,
        out: &mut String,
        problem: &mut Option<&'static str>,
    ) -> Result<(), ParseErr> {
        let Some(esc) = self.bump() else {
            return Err(self.fail("Unfinished string", true));
        };
        match esc {
            '"' | '\\' | '/' => {
                if problem.is_none() {
                    out.push(esc);
                }
            }
            'b' => push_if_ok(out, problem, '\u{0008}'),
            'f' => push_if_ok(out, problem, '\u{000c}'),
            'n' => push_if_ok(out, problem, '\n'),
            'r' => push_if_ok(out, problem, '\r'),
            't' => push_if_ok(out, problem, '\t'),
            'u' => self.parse_u(out, problem)?,
            _ => note(problem, "Invalid escape"),
        }
        Ok(())
    }

    fn parse_u(
        &mut self,
        out: &mut String,
        problem: &mut Option<&'static str>,
    ) -> Result<(), ParseErr> {
        let Some(unit) = self.read_hex_unit(problem, INVALID_U, BAD_U_CHARS)? else {
            return Ok(());
        };
        if (0xD800..=0xDBFF).contains(&unit) {
            return self.parse_low_surrogate(unit, out, problem);
        }
        if problem.is_some() {
            return Ok(());
        }
        if (0xDC00..=0xDFFF).contains(&unit) {
            // jq keeps a lone low surrogate and prints U+FFFD.
            out.push('\u{FFFD}');
            return Ok(());
        }
        if let Some(c) = char::from_u32(unit) {
            out.push(c);
        }
        Ok(())
    }

    fn parse_low_surrogate(
        &mut self,
        high: u32,
        out: &mut String,
        problem: &mut Option<&'static str>,
    ) -> Result<(), ParseErr> {
        if self.eof() {
            return Err(self.fail("Unfinished string", true));
        }
        if !self.src[self.index..].starts_with("\\u") {
            note(problem, SURROGATE);
            return Ok(());
        }
        self.bump();
        self.bump();
        let Some(low) = self.read_hex_unit(problem, SURROGATE, SURROGATE)? else {
            return Ok(());
        };
        if !(0xDC00..=0xDFFF).contains(&low) {
            note(problem, SURROGATE);
            return Ok(());
        }
        if problem.is_some() {
            return Ok(());
        }
        let scalar = 0x10000 + (((high - 0xD800) << 10) | (low - 0xDC00));
        if let Some(c) = char::from_u32(scalar) {
            out.push(c);
        }
        Ok(())
    }

    /// Read four hex digits. A closing quote ends the attempt early. End of
    /// input while the string is still open is an unfinished string.
    fn read_hex_unit(
        &mut self,
        problem: &mut Option<&'static str>,
        early: &'static str,
        bad: &'static str,
    ) -> Result<Option<u32>, ParseErr> {
        let mut value = 0u32;
        let mut bad_digit = false;
        for _ in 0..4 {
            match self.peek() {
                None => return Err(self.fail("Unfinished string", true)),
                Some('"') => {
                    note(problem, early);
                    return Ok(None);
                }
                Some(c) => {
                    self.bump();
                    match c.to_digit(16) {
                        Some(digit) => value = (value << 4) | digit,
                        None => bad_digit = true,
                    }
                }
            }
        }
        if bad_digit {
            note(problem, bad);
            return Ok(None);
        }
        Ok(Some(value))
    }
}

const CONTROL: &str =
    "Invalid string: control characters from U+0000 through U+001F must be escaped";
const INVALID_U: &str = "Invalid \\uXXXX escape";
const BAD_U_CHARS: &str = "Invalid characters in \\uXXXX escape";
const SURROGATE: &str = "Invalid \\uXXXX\\uXXXX surrogate pair escape";

fn note(problem: &mut Option<&'static str>, phrase: &'static str) {
    if problem.is_none() {
        *problem = Some(phrase);
    }
}

fn push_if_ok(out: &mut String, problem: &Option<&'static str>, c: char) {
    if problem.is_none() {
        out.push(c);
    }
}

fn invalid_phrase(token: &str) -> &'static str {
    // `t` and `f` start true/false. `nu` has already chosen null over nan.
    let literal = token.starts_with('t') || token.starts_with('f') || token.starts_with("nu");
    if literal {
        "Invalid literal"
    } else {
        "Invalid numeric literal"
    }
}

fn interpret_atom(token: &str) -> Option<Value> {
    let mut parser = Parser::new(token);
    let value = parser.parse_scalar().ok()?;
    if parser.eof() {
        Some(value)
    } else {
        None
    }
}

impl<'a> Parser<'a> {
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
        if !self.src[self.index..].starts_with(text) {
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
        if self.src[self.index..].starts_with("null") {
            return self.literal("null", Value::Null);
        }
        self.parse_nan()
    }

    fn parse_nan(&mut self) -> Result<Value, ()> {
        if self.take_nonfinite_word("nan") {
            return Ok(Value::Number("null".to_string()));
        }
        Err(())
    }

    fn parse_infinity(&mut self, negative: bool) -> Result<Value, ()> {
        if self.take_nonfinite_word("infinity") || self.take_nonfinite_word("inf") {
            return Ok(Value::Number(number::infinite(negative)));
        }
        Err(())
    }

    fn take_nonfinite_word(&mut self, word: &str) -> bool {
        let rest = &self.src[self.index..];
        if rest.len() < word.len() || !rest[..word.len()].eq_ignore_ascii_case(word) {
            return false;
        }
        let after = self.index + word.len();
        let boundary = match self.src[after..].chars().next() {
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
