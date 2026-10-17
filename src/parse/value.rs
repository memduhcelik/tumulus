//! Arrays, objects, and the decision of which value comes next.

use super::parser::{ParseErr, Parser};
use crate::value::Value;

impl<'a> Parser<'a> {
    pub(in crate::parse) fn parse_value(&mut self) -> Result<Value, ParseErr> {
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
            return Ok(Value::array(Vec::new()));
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
        Ok(Value::array(items))
    }

    fn parse_object(&mut self) -> Result<Value, ParseErr> {
        self.skip_ws();
        if self.peek() == Some('}') {
            self.bump();
            return Ok(Value::object(Vec::new()));
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
        Ok(Value::object(items))
    }

    pub(in crate::parse) fn expect_colon(&mut self) -> Result<(), ParseErr> {
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

    pub(in crate::parse) fn bad_key(&mut self) -> ParseErr {
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
    pub(in crate::parse) fn expected_separator(&mut self) -> ParseErr {
        self.expected_separator_phrase("Expected separator between values")
    }

    /// `--stream` says "Expected a separator between values".
    pub(in crate::parse) fn expected_stream_separator(&mut self) -> ParseErr {
        self.expected_separator_phrase("Expected a separator between values")
    }

    fn expected_separator_phrase(&mut self, phrase: &'static str) -> ParseErr {
        match self.peek() {
            Some('[' | '{') => {
                self.bump();
                self.fail(phrase, false)
            }
            Some('"') => match self.parse_string() {
                Err(err) => err,
                Ok(_) => self.fail(phrase, false),
            },
            Some(_) => match self.scan_atom() {
                Err(err) => err,
                Ok(_) => {
                    if self.eof() {
                        self.fail(phrase, true)
                    } else {
                        self.bump();
                        self.fail(phrase, false)
                    }
                }
            },
            None => self.fail("Unfinished JSON term", true),
        }
    }
}
