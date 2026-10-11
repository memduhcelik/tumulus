//! JSON strings, escapes, and the jq messages for a bad one.

use super::parser::{ParseErr, Parser};

const CONTROL: &str =
    "Invalid string: control characters from U+0000 through U+001F must be escaped";
const INVALID_U: &str = "Invalid \\uXXXX escape";
const BAD_U_CHARS: &str = "Invalid characters in \\uXXXX escape";
const SURROGATE: &str = "Invalid \\uXXXX\\uXXXX surrogate pair escape";

impl<'a> Parser<'a> {
    pub(in crate::parse) fn parse_string(&mut self) -> Result<String, ParseErr> {
        self.bump();
        // Raw content bytes, with escapes already turned into their bytes.
        // Invalid UTF-8 is replaced once the string has its closing quote,
        // because a short tail is one U+FFFD for whatever bytes are left.
        let mut raw = Vec::new();
        let mut problem = None;
        loop {
            let Some(byte) = self.take_byte() else {
                return Err(self.fail("Unfinished string", true));
            };
            match byte {
                b'"' => {
                    if let Some(phrase) = problem {
                        return Err(self.fail(phrase, false));
                    }
                    return Ok(super::parser::sanitize_content(&raw));
                }
                b'\\' => self.parse_escape(&mut raw, &mut problem)?,
                b if b < 0x20 => note(&mut problem, CONTROL),
                b => {
                    if problem.is_none() {
                        raw.push(b);
                    }
                }
            }
        }
    }

    fn parse_escape(
        &mut self,
        out: &mut Vec<u8>,
        problem: &mut Option<&'static str>,
    ) -> Result<(), ParseErr> {
        let Some(esc) = self.bump() else {
            return Err(self.fail("Unfinished string", true));
        };
        match esc {
            '"' | '\\' | '/' => push_char(out, problem, esc),
            'b' => push_char(out, problem, '\u{0008}'),
            'f' => push_char(out, problem, '\u{000c}'),
            'n' => push_char(out, problem, '\n'),
            'r' => push_char(out, problem, '\r'),
            't' => push_char(out, problem, '\t'),
            'u' => self.parse_u(out, problem)?,
            _ => note(problem, "Invalid escape"),
        }
        Ok(())
    }

    fn parse_u(
        &mut self,
        out: &mut Vec<u8>,
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
            push_char(out, problem, '\u{FFFD}');
            return Ok(());
        }
        if let Some(c) = char::from_u32(unit) {
            push_char(out, problem, c);
        }
        Ok(())
    }

    fn parse_low_surrogate(
        &mut self,
        high: u32,
        out: &mut Vec<u8>,
        problem: &mut Option<&'static str>,
    ) -> Result<(), ParseErr> {
        if self.eof() {
            return Err(self.fail("Unfinished string", true));
        }
        if !self.src[self.index..].starts_with(b"\\u") {
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
            push_char(out, problem, c);
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

fn note(problem: &mut Option<&'static str>, phrase: &'static str) {
    if problem.is_none() {
        *problem = Some(phrase);
    }
}

fn push_char(out: &mut Vec<u8>, problem: &Option<&'static str>, c: char) {
    if problem.is_none() {
        let mut buf = [0u8; 4];
        out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
    }
}
