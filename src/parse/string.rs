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
